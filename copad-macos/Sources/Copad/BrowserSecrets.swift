import CopadCore
import Foundation

/// Glue between the credential INDEX (metadata, owned by `copad-core`) and the
/// SECRET STORE (the platform keychain).
///
/// The split is the whole design: the index is a list of `CredentialRef`, a type
/// with no field that can hold secret material, and it is what an agent may
/// read. The secret exists only in the keychain, and the only code path that
/// ever holds its plaintext is `BrowserTab.fillCredential`, which hands it
/// straight to the page.
@MainActor
enum BrowserSecrets {
    static func list(origin: String?) -> [CredentialRef] {
        BrowserFFI.listCredentials(origin: origin)
    }

    static func find(id: String) -> CredentialRef? {
        BrowserFFI.listCredentials(origin: nil).first { $0.id == id }
    }

    /// Read the password out of a PROTECTED page and store it.
    ///
    /// The value is read natively, inside this process, from a document
    /// automation was never in. It is not an RPC parameter — an agent asking
    /// copad to remember a password never handles the password. And it is
    /// refused outright on an unprotected tab, because on such a page the value
    /// was already readable through `query` and `execute_js` before this was
    /// ever called, so "saving it securely" afterwards would be theatre.
    static func saveFromPage(
        tab: BrowserTab,
        username: String,
        completion: @escaping (Result<CredentialRef, RPCError>) -> Void,
    ) {
        guard tab.tabMode == .protected else {
            completion(.failure(RPCError(
                code: "requires_protected",
                message: "a password may only be captured from a protected tab",
            )))
            return
        }
        let origin = BrowserFFI.canonicalize(tab.currentURL, policy: "origin").url
        guard !origin.isEmpty else {
            completion(.failure(RPCError(
                code: "origin_mismatch",
                message: "the tab is not on an http(s) origin",
            )))
            return
        }
        let generationAtStart = tab.documentGeneration
        // The page reports its own origin alongside the value. Capturing the
        // origin BEFORE the async evaluation and trusting it afterwards would
        // let a navigation mid-read file another site's password under this
        // one — and it would then be fillable here.
        // Whether this credential already exists decides what a failed index
        // write may roll back.
        let existedBefore = find(id: "\(origin)/\(username)") != nil
        let js = """
        (() => {
            const el = document.querySelector('input[type="password"]');
            if (!el || !el.value) return JSON.stringify(null);
            return JSON.stringify({ value: el.value, origin: location.origin });
        })()
        """
        let box = SendableBox<(Result<CredentialRef, RPCError>) -> Void>(completion)
        tab.executeJS(js) { result, _ in
            Task { @MainActor in
                guard let raw = result as? String,
                      let data = raw.data(using: .utf8),
                      let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let secret = obj["value"] as? String, !secret.isEmpty,
                      let pageOrigin = obj["origin"] as? String
                else {
                    box.value(.failure(RPCError(
                        code: "no_target",
                        message: "no non-empty password field on this page",
                    )))
                    return
                }
                guard tab.documentGeneration == generationAtStart,
                      tab.tabMode == .protected,
                      pageOrigin.caseInsensitiveCompare(origin) == .orderedSame
                else {
                    box.value(.failure(RPCError(
                        code: "document_changed",
                        message: "the tab changed while the password was being read",
                    )))
                    return
                }
                // The id carries the WHOLE origin, scheme included. Dropping it
                // meant `http://example.com` and `https://example.com` shared a
                // keychain account and an index row, so saving one silently
                // destroyed the other — while `validate_fill` still treated the
                // two origins as different and refused to use it.
                let ref = CredentialRef(
                    id: "\(origin)/\(username)",
                    origin: origin,
                    username: username,
                    slot: "password",
                    createdAt: UInt64(Date().timeIntervalSince1970),
                )
                BrowserKeychain.save(
                    id: ref.id, secret: secret, origin: origin, slot: "password",
                ) { saved in
                    switch saved {
                    case let .failure(err):
                        box.value(.failure(BrowserTab.keychainError(err)))
                    case .success:
                        // Index LAST: an index entry whose secret was never
                        // stored would offer the user a credential that cannot
                        // be filled.
                        guard BrowserFFI.upsertCredential(ref) else {
                            // Roll back ONLY a credential this call created.
                            // Deleting unconditionally destroyed the stored
                            // password of an existing credential whose index
                            // row was already there and still pointing at it —
                            // a failed save that lost the thing it was saving.
                            if !existedBefore {
                                BrowserKeychain.delete(id: ref.id)
                            }
                            box.value(.failure(RPCError(
                                code: "index_write_failed",
                                message: existedBefore
                                    ? "could not update the credential record; the stored secret was left as it is"
                                    : "could not record the credential; the secret was rolled back",
                            )))
                            return
                        }
                        box.value(.success(ref))
                    }
                }
            }
        }
    }

    /// Forget a credential. Keychain FIRST: an index entry removed while its
    /// secret survives is an orphan nothing can ever clean up.
    static func delete(
        id: String,
        completion: @escaping (Result<Bool, RPCError>) -> Void,
    ) {
        let box = SendableBox<(Result<Bool, RPCError>) -> Void>(completion)
        BrowserKeychain.delete(id: id) { result in
            if case let .failure(err) = result, case .unavailable = err {
                box.value(.failure(BrowserTab.keychainError(err)))
                return
            }
            box.value(.success(BrowserFFI.removeCredential(id: id)))
        }
    }
}

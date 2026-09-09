import Foundation
import Security

/// The one place a website password exists outside the page.
///
/// **There is no plaintext fallback.** Decision #54 put Slack bot tokens in a
/// `0600` file because they are revocable in one click and read by a background
/// plugin that cannot answer a Keychain prompt. A user's website passwords are
/// neither, and a `0600` file is readable by the very same-user agent this whole
/// design exists to keep them away from. An unavailable Keychain therefore
/// DISABLES the feature rather than downgrading it.
///
/// Prefers the **data-protection keychain** (`kSecUseDataProtectionKeychain`)
/// and falls back to the legacy file keychain when the app is not entitled for
/// it.
///
/// The preference exists because of decision #54: the legacy keychain gates
/// access on an ACL plus a partition list bound to the calling binary's cdhash,
/// so a rebuilt binary reading an item the previous build saved can PROMPT —
/// and a prompt nobody answers is an indefinite block, not an error. The
/// data-protection keychain scopes items to the signing identity instead, with
/// no prompt and no partition list.
///
/// The fallback exists because the data-protection keychain requires a
/// `keychain-access-groups` entitlement, which needs a team identifier — a
/// Developer ID release build has one, a locally self-signed dev build does
/// not, and both `SecItem` paths then fail with `errSecMissingEntitlement`
/// (-34018). Adding the entitlement to a self-signed bundle is not a way out:
/// AMFI kills the process on launch (`Killed: 9`), which was measured, not
/// assumed. Refusing to work in a dev build would mean this code path was never
/// exercised outside a release — the last place to find out it is wrong.
///
/// The last tier is the pre-unified `SecKeychain*` API. It is deprecated and it
/// is what the `security` CLI uses; it talks to the file keychain directly and
/// needs no entitlement at all.
///
/// **The legacy caveat is real and not papered over:** an item saved by one
/// build and read by a later rebuild may prompt, because the cdhash changed and
/// the ACL is bound to it (decision #54). Within one run — save and read by the
/// same binary — it does not.
enum BrowserKeychain {
    /// How long a keychain call may take before it is treated as unavailable.
    ///
    /// **This is not a nicety, it is the decision #54 failure.** A legacy
    /// keychain read whose ACL does not list the calling binary raises a
    /// SecurityAgent prompt, and a prompt nobody answers is an INDEFINITE
    /// block. Run on the main actor — where every RPC handler lives — that
    /// freezes the entire GUI, not just the credential call: measured here as a
    /// socket that stopped answering and a window that stopped drawing.
    ///
    /// So every keychain operation runs off the main thread and is abandoned
    /// after this long, surfacing as `secret_backend_unavailable`. A feature
    /// that reports itself unavailable is recoverable; an app that stops
    /// responding is not.
    private static let timeout: TimeInterval = 3

    /// Run a keychain operation off the main thread, bounded, and deliver the
    /// result back on the main actor.
    ///
    /// **Asynchronous, not "off-thread but awaited".** A first version ran the
    /// work on a queue and then blocked on a semaphore — which pushed the freeze
    /// from indefinite down to three seconds but did not remove it: every
    /// credential caller is on the main actor, so the whole GUI still stopped
    /// for three seconds per call. Bounding a freeze is not the same as not
    /// freezing.
    ///
    /// The work is NOT cancelled on timeout — there is no way to cancel a
    /// blocked `SecItemCopyMatching` — it is abandoned, and a late completion
    /// finds `delivered` already true and does nothing.
    private static func bounded<T: Sendable>(
        _ fallback: T,
        _ body: @escaping @Sendable () -> T,
        then deliver: @escaping @MainActor (T) -> Void,
    ) {
        let state = UncheckedBox<Bool>(false)
        let lock = NSLock()
        let finish: @Sendable (T) -> Void = { value in
            lock.lock()
            let already = state.value
            state.value = true
            lock.unlock()
            guard !already else { return }
            Task { @MainActor in deliver(value) }
        }
        DispatchQueue.global(qos: .userInitiated).async { finish(body()) }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + timeout) {
            lock.lock()
            let already = state.value
            lock.unlock()
            guard !already else { return }
            FileHandle.standardError.write(Data(
                "[copad-keychain] a keychain call did not return within \(timeout)s — most likely an unanswered access prompt; treating the backend as unavailable\n".utf8,
            ))
            finish(fallback)
        }
    }

    /// Minimal mutable box so the escaping closure has somewhere to put its
    /// result. Access is serialized by the semaphore.
    private final class UncheckedBox<T>: @unchecked Sendable {
        var value: T
        init(_ value: T) { self.value = value }
    }

    /// One service for every browser credential; the credential id is the
    /// account. Deliberately distinct from the plugins' service names so a
    /// keychain audit can tell "a website password copad saved for me" apart
    /// from "an API token a plugin holds".
    private static let service = "com.marshall.copad.browser"

    enum Failure: Error {
        /// The backend itself is unusable — not "no such credential".
        case unavailable(String)
        case notFound
    }

    /// What the keychain holds for one credential.
    ///
    /// The origin and slot travel WITH the secret, not alongside it in the
    /// index. The index is a plain JSON file that anything running as the user
    /// — including the agent — can rewrite; pointing an existing entry at an
    /// attacker-chosen origin would otherwise keep its keychain account (the
    /// lookup is by id alone) while making both the validator and the
    /// injection's own origin check accept the new destination. The keychain is
    /// the trusted store, so the binding lives there.
    struct Stored: Sendable {
        let secret: String
        let origin: String
        let slot: String
    }

    static func save(
        id: String,
        secret: String,
        origin: String,
        slot: String,
        then deliver: @escaping @MainActor (Result<Void, Failure>) -> Void,
    ) {
        let envelope: [String: String] = ["secret": secret, "origin": origin, "slot": slot]
        guard let data = try? JSONSerialization.data(withJSONObject: envelope) else {
            Task { @MainActor in deliver(.failure(.unavailable("credential could not be encoded"))) }
            return
        }
        bounded(.failure(.unavailable("keychain timed out")), {
            saveBlocking(id: id, data: data)
        }, then: deliver)
    }

    static func read(
        id: String,
        then deliver: @escaping @MainActor (Result<Stored, Failure>) -> Void,
    ) {
        bounded(.failure(.unavailable("keychain timed out")), { readBlocking(id: id) }, then: deliver)
    }

    static func delete(
        id: String,
        then deliver: @escaping @MainActor (Result<Void, Failure>) -> Void = { _ in },
    ) {
        bounded(.failure(.unavailable("keychain timed out")), { deleteBlocking(id: id) }, then: deliver)
    }

    // MARK: - Blocking implementations (always off the main thread)

    private static func saveBlocking(id: String, data: Data) -> Result<Void, Failure> {
        // UPDATE first, add only if absent.
        //
        // Delete-then-add looked simpler and is destructive: re-saving a
        // credential removed the stored secret and then, if the add failed for
        // any reason, the old password was gone for good while its index entry
        // still promised it. An update that matches nothing is an ordinary
        // `errSecItemNotFound`, which is exactly the signal to add.
        let updated = withKeychain { dataProtection in
            SecItemUpdate(
                baseQuery(id: id, dataProtection: dataProtection) as CFDictionary,
                [kSecValueData as String: data] as CFDictionary,
            )
        }
        if updated == errSecSuccess { return .success(()) }
        if updated != errSecItemNotFound, updated != errSecMissingEntitlement {
            return .failure(.unavailable(describe(updated)))
        }
        let status = withKeychain { dataProtection in
            var attrs = baseQuery(id: id, dataProtection: dataProtection)
            attrs[kSecValueData as String] = data
            // Available whenever the device is unlocked, and never synced to
            // iCloud: a browser password copad holds is for THIS machine, and
            // syncing it would widen the blast radius for no benefit here.
            attrs[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked
            return SecItemAdd(attrs as CFDictionary, nil)
        }
        if status == errSecMissingEntitlement {
            return legacySave(id: id, data: data)
        }
        guard status == errSecSuccess else {
            return .failure(.unavailable(describe(status)))
        }
        return .success(())
    }

    /// The returned `String` is the ONLY place the plaintext exists in this
    /// process, and the caller hands it straight to the page and drops it. It
    /// never crosses an RPC boundary.
    private static func readBlocking(id: String) -> Result<Stored, Failure> {
        var out: CFTypeRef?
        let status = withKeychain { dataProtection in
            var query = baseQuery(id: id, dataProtection: dataProtection)
            query[kSecReturnData as String] = true
            query[kSecMatchLimit as String] = kSecMatchLimitOne
            return SecItemCopyMatching(query as CFDictionary, &out)
        }
        if status == errSecMissingEntitlement { return legacyRead(id: id) }
        if status == errSecItemNotFound { return .failure(.notFound) }
        guard status == errSecSuccess, let data = out as? Data else {
            return .failure(.unavailable(describe(status)))
        }
        return decode(data)
    }

    private static func deleteBlocking(id: String) -> Result<Void, Failure> {
        let status = withKeychain { dataProtection in
            SecItemDelete(baseQuery(id: id, dataProtection: dataProtection) as CFDictionary)
        }
        if status == errSecSuccess || status == errSecItemNotFound { return .success(()) }
        if status == errSecMissingEntitlement { return legacyDelete(id: id) }
        return .failure(.unavailable(describe(status)))
    }

    private static func baseQuery(id: String, dataProtection: Bool) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: id,
        ]
        if dataProtection {
            q[kSecUseDataProtectionKeychain as String] = true
        }
        return q
    }

    /// Which keychain this process ended up on. Resolved once, on the first
    /// operation, by trying the preferred one and watching for
    /// `errSecMissingEntitlement`.
    ///
    /// Every keychain operation now runs on a global queue, so concurrent
    /// credential requests reach this from several threads at once — a genuine
    /// data race, which `nonisolated(unsafe)` would only have hidden from the
    /// compiler. Guarded by `backendLock`.
    private nonisolated(unsafe) static var useDataProtectionStorage: Bool?
    private static let backendLock = NSLock()

    private static var useDataProtection: Bool? {
        get {
            backendLock.lock()
            defer { backendLock.unlock() }
            return useDataProtectionStorage
        }
        set {
            backendLock.lock()
            useDataProtectionStorage = newValue
            backendLock.unlock()
        }
    }

    /// Run `body` against the preferred keychain, retrying on the legacy one
    /// exactly once if the app turns out not to be entitled.
    private static func withKeychain(_ body: (Bool) -> OSStatus) -> OSStatus {
        if let known = useDataProtection {
            return body(known)
        }
        let status = body(true)
        if status == errSecMissingEntitlement || status == errSecParam {
            let fallback = body(false)
            // Only remember the fallback if it actually worked; a failure for
            // some other reason should not pin us to the legacy keychain.
            if fallback == errSecSuccess || fallback == errSecItemNotFound {
                useDataProtection = false
            }
            return fallback
        }
        if status == errSecSuccess || status == errSecItemNotFound {
            useDataProtection = true
        }
        return status
    }

    // MARK: - Pre-unified Keychain API
    //
    // Entitlement-free, and the only tier that works in a self-signed dev
    // build. Kept behind the modern paths so a properly signed release never
    // touches it.
    //
    // `SecKeychain*` is deprecated and the compiler says so on each call below.
    // The warnings are deliberate and left visible: the alternative is
    // annotating these helpers `@available(deprecated:)`, which would be a lie
    // — they are not deprecated, the API they wrap is — and would only move the
    // same warnings to their call sites.

    private static func legacySave(id: String, data: Data) -> Result<Void, Failure> {
        // Same rule as the modern path: modify in place when the item exists,
        // so a failed write cannot destroy the password that was already there.
        let svc = Array(self.service.utf8)
        let acct = Array(id.utf8)
        var existing: SecKeychainItem?
        let found = SecKeychainFindGenericPassword(
            nil,
            UInt32(svc.count), svc.map { CChar(bitPattern: $0) },
            UInt32(acct.count), acct.map { CChar(bitPattern: $0) },
            nil, nil, &existing,
        )
        if found == errSecSuccess, let existing {
            let bytes = [UInt8](data)
            let status = SecKeychainItemModifyAttributesAndData(
                existing, nil, UInt32(bytes.count), bytes,
            )
            guard status == errSecSuccess else {
                return .failure(.unavailable(describe(status)))
            }
            return .success(())
        }
        let service = Array(service.utf8)
        let account = Array(id.utf8)
        let bytes = [UInt8](data)
        let status = SecKeychainAddGenericPassword(
            nil,
            UInt32(service.count), service.map { CChar(bitPattern: $0) },
            UInt32(account.count), account.map { CChar(bitPattern: $0) },
            UInt32(bytes.count), bytes,
            nil,
        )
        guard status == errSecSuccess else {
            return .failure(.unavailable(describe(status)))
        }
        return .success(())
    }

    private static func legacyRead(id: String) -> Result<Stored, Failure> {
        let service = Array(self.service.utf8)
        let account = Array(id.utf8)
        var length: UInt32 = 0
        var bytes: UnsafeMutableRawPointer?
        let status = SecKeychainFindGenericPassword(
            nil,
            UInt32(service.count), service.map { CChar(bitPattern: $0) },
            UInt32(account.count), account.map { CChar(bitPattern: $0) },
            &length, &bytes, nil,
        )
        if status == errSecItemNotFound { return .failure(.notFound) }
        guard status == errSecSuccess, let bytes else {
            return .failure(.unavailable(describe(status)))
        }
        defer { SecKeychainItemFreeContent(nil, bytes) }
        return decode(Data(bytes: bytes, count: Int(length)))
    }

    private static func legacyDelete(id: String) -> Result<Void, Failure> {
        let service = Array(self.service.utf8)
        let account = Array(id.utf8)
        var item: SecKeychainItem?
        let found = SecKeychainFindGenericPassword(
            nil,
            UInt32(service.count), service.map { CChar(bitPattern: $0) },
            UInt32(account.count), account.map { CChar(bitPattern: $0) },
            nil, nil, &item,
        )
        if found == errSecItemNotFound { return .success(()) }
        guard found == errSecSuccess, let item else {
            return .failure(.unavailable(describe(found)))
        }
        let status = SecKeychainItemDelete(item)
        guard status == errSecSuccess else {
            return .failure(.unavailable(describe(status)))
        }
        return .success(())
    }

    private static func decode(_ data: Data) -> Result<Stored, Failure> {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: String],
              let secret = obj["secret"], let origin = obj["origin"], let slot = obj["slot"]
        else {
            // An item written by an older build, or by hand, has no envelope.
            // Refuse it rather than guessing an origin — a credential whose
            // binding cannot be established must not be fillable anywhere.
            return .failure(.unavailable("stored credential has no origin binding"))
        }
        return .success(Stored(secret: secret, origin: origin, slot: slot))
    }

    private static func describe(_ status: OSStatus) -> String {
        let message = SecCopyErrorMessageString(status, nil) as String? ?? "unknown"
        return "\(message) (OSStatus \(status))"
    }
}

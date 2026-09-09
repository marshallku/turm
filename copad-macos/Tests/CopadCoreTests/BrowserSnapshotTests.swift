@testable import CopadCore
import XCTest

/// The Swift half of the cross-language schema check. The Rust half is
/// `copad_core::browser::tabs::tests::the_shared_swift_fixture_round_trips`,
/// and **the literal below must stay byte-identical to the one there.**
///
/// It is fully populated on purpose: a fixture with default values would decode
/// and re-encode unchanged even if one side gained a field the other never
/// learned, so it would prove nothing. Keys are sorted on both sides so
/// byte-equality is well defined.
///
/// This pins TODAY's schema. It cannot detect a future `#[serde(default)]`
/// Rust field that is omitted when empty — the rule that closes that gap is
/// procedural and stated in `BrowserSnapshot.swift`: a schema change updates
/// this fixture in the same commit.
let sharedPaneFixture = """
{"active":1,"profile":"work","tabs":[{"history_depth":3,"history_generation":7,"id":"tab-a","last_active":1725840000,"pinned":true,"scroll_y":412.5,"title":"Pull Request #42","url":"https://github.com/o/r/pull/42"},{"id":"tab-b","last_active":1725840001,"url":"https://example.com"}]}
"""

final class BrowserSnapshotTests: XCTestCase {
    private func sortedEncoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return e
    }

    func testSharedFixtureRoundTripsByteIdentically() throws {
        let data = Data(sharedPaneFixture.utf8)
        let pane = try JSONDecoder().decode(BrowserPaneSnap.self, from: data)

        XCTAssertEqual(pane.active, 1)
        XCTAssertEqual(pane.profile, "work")
        XCTAssertEqual(pane.tabs.count, 2)
        XCTAssertEqual(pane.tabs[0].historyGeneration, 7)
        XCTAssertEqual(pane.tabs[0].historyDepth, 3)
        XCTAssertEqual(pane.tabs[0].scrollY, 412.5)
        XCTAssertTrue(pane.tabs[0].pinned)
        XCTAssertEqual(pane.tabs[0].title, "Pull Request #42")

        let reencoded = try sortedEncoder().encode(pane)
        XCTAssertEqual(String(decoding: reencoded, as: UTF8.self), sharedPaneFixture)
    }

    func testAPaneDecodesWithTheSameDefaultsRustUses() throws {
        // Rust marks `active` and `profile` `#[serde(default)]`. Synthesized
        // Swift decoding would make both REQUIRED, so a pane omitting them
        // would fail to decode, `Session` would discard it, and restore would
        // mint a new tab id — silently losing the tab's identity.
        let minimal = #"{"tabs":[{"id":"tab-a","url":"https://example.com"}]}"#
        let pane = try JSONDecoder().decode(BrowserPaneSnap.self, from: Data(minimal.utf8))
        XCTAssertEqual(pane.active, 0)
        XCTAssertEqual(pane.profile, BrowserPaneSnap.defaultProfile)
        XCTAssertEqual(pane.tabs.first?.id, "tab-a")

        // And an entirely empty object is "no browser state", not an error.
        let empty = try JSONDecoder().decode(BrowserPaneSnap.self, from: Data("{}".utf8))
        XCTAssertTrue(empty.tabs.isEmpty)
        XCTAssertEqual(empty.active, 0)
    }

    func testAnEmptyTabOmitsEveryOptionalFieldLikeSerdeDoes() throws {
        // A pane with no browser state must serialize the way the pre-Workbench
        // binary wrote it, or an older copad can no longer read the file.
        let tab = BrowserTabSnap(id: "t1", url: "https://e.com")
        let json = String(decoding: try sortedEncoder().encode(tab), as: UTF8.self)
        XCTAssertEqual(json, #"{"id":"t1","last_active":0,"url":"https://e.com"}"#)
    }

    // MARK: - resolveURL

    func testASnapshotBeforeTheFirstLoadKeepsThePendingRestoreURL() {
        // The bug this exists to prevent: autosave fires on a timer while a
        // restored pane is still loading, WKWebView.url is nil, and the pane
        // gets written out as "" — erased before it ever opened.
        XCTAssertEqual(
            BrowserSnapshot.resolveURL(live: nil, pending: "https://github.com"),
            "https://github.com",
        )
        XCTAssertEqual(
            BrowserSnapshot.resolveURL(live: "", pending: "https://github.com"),
            "https://github.com",
        )
        // The blank placeholder page must not win over a pending restore either.
        XCTAssertEqual(
            BrowserSnapshot.resolveURL(live: "about:blank", pending: "https://github.com"),
            "https://github.com",
        )
    }

    func testALiveURLWinsOnceTheNavigationHasCommitted() {
        // The caller clears `pending` on a main-frame commit, so "committed"
        // means pending is nil — not that live merely has a value.
        XCTAssertEqual(
            BrowserSnapshot.resolveURL(live: "https://github.com/o/r", pending: nil),
            "https://github.com/o/r",
        )
    }

    func testAPendingDestinationBeatsAStaleLiveURL() {
        // Load A, navigate to an unreachable B: WebKit keeps reporting A, but
        // B is the newer intent. Preferring live would silently discard the
        // navigation the user asked for.
        XCTAssertEqual(
            BrowserSnapshot.resolveURL(live: "https://a.example", pending: "https://b.example"),
            "https://b.example",
        )
    }

    func testAPaneWithNothingAtAllResolvesToEmpty() {
        XCTAssertEqual(BrowserSnapshot.resolveURL(live: nil, pending: nil), "")
        XCTAssertEqual(BrowserSnapshot.resolveURL(live: "about:blank", pending: nil), "")
    }

    func testTheActiveTabIsTheOneRestoreShouldAdopt() {
        // The rule `WebViewController.init` follows: identity and URL come from
        // the ACTIVE tab. Taking `tabs.first` would hand tab A's identity to
        // tab B's page and persist that wrong pairing on the next autosave.
        let pane = try! JSONDecoder().decode(
            BrowserPaneSnap.self,
            from: Data(sharedPaneFixture.utf8),
        )
        XCTAssertEqual(pane.active, 1)
        XCTAssertEqual(pane.tabs[pane.active].id, "tab-b")
        XCTAssertEqual(pane.tabs[pane.active].url, "https://example.com")
        // ...and it differs from the first tab, so the distinction is real.
        XCTAssertNotEqual(pane.tabs[0].id, pane.tabs[pane.active].id)
    }

    func testPaneSnapshotReusesTheTabIdentitySoHistorySurvivesARestart() {
        let snap = BrowserSnapshot.pane(tabID: "tab-a", url: "https://e.com", title: "E")
        XCTAssertEqual(snap.tabs.count, 1)
        XCTAssertEqual(snap.tabs[0].id, "tab-a")
        XCTAssertEqual(snap.active, 0)
        XCTAssertEqual(snap.profile, BrowserPaneSnap.defaultProfile)
    }

    func testAFreshTabIdIsAcceptableToTheRustCharsetRule() {
        // Swift deliberately has NO id validator — that rule is enforced once,
        // in Rust, and a restored pane only reaches a controller through
        // `normalize`. What Swift must still guarantee is that the ids it MINTS
        // would survive that rule.
        let id = BrowserSnapshot.freshTabID()
        XCTAssertFalse(id.isEmpty)
        XCTAssertLessThanOrEqual(id.count, 64)
        XCTAssertTrue(id.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") })
    }

    // MARK: - FFI decode, fail-closed

    func testCanonicalDecodeFallsBackToEmptyOnEveryFailureShape() {
        for bad in [nil, "{not json", "{}", #"{"url":42}"#] as [String?] {
            XCTAssertEqual(BrowserFFIDecode.canonical(bad), .unavailable, "input: \(bad ?? "nil")")
        }
        let ok = BrowserFFIDecode.canonical(#"{"url":"https://e.com","persist_title":true}"#)
        XCTAssertEqual(ok.url, "https://e.com")
        XCTAssertTrue(ok.persistTitle)
    }

    func testAMissingPersistTitleFlagMeansDoNotPersistIt() {
        // Restrictive direction: a page title carries the same exposure as a
        // URL, so an FFI response that omits the flag must not be read as
        // permission.
        let d = BrowserFFIDecode.canonical(#"{"url":"https://e.com"}"#)
        XCTAssertEqual(d.url, "https://e.com")
        XCTAssertFalse(d.persistTitle)
        XCTAssertFalse(BrowserCanonicalURL.unavailable.persistTitle)
        XCTAssertEqual(BrowserCanonicalURL.unavailable.url, "")
    }

    func testNormalizedDecodeFallsBackToNilRatherThanAnUnvalidatedPane() {
        XCTAssertNil(BrowserFFIDecode.normalized(nil))
        XCTAssertNil(BrowserFFIDecode.normalized("{not json"))
        XCTAssertNil(BrowserFFIDecode.normalized("{}"))
        XCTAssertNil(BrowserFFIDecode.normalized(#"{"pane":42}"#))

        let ok = BrowserFFIDecode.normalized(#"{"pane":{"tabs":[{"id":"a","url":"https://e.com"}],"active":0,"profile":"default"},"repairs":{}}"#)
        XCTAssertEqual(ok?.tabs.first?.id, "a")
    }

    func testAuthorizationDecodeRefusesWhenItCannotBeEvaluated() {
        // The whole point: an FFI failure must not become "permit everything".
        for bad in [nil, "{not json", "{}", #"{"allowed":"yes"}"#] as [String?] {
            let decision = BrowserFFIDecode.authorization(bad)
            XCTAssertFalse(decision.allowed, "input: \(bad ?? "nil")")
            XCTAssertTrue(decision.opaqueWrite, "input: \(bad ?? "nil")")
            XCTAssertEqual(decision.code, "authorization_unavailable")
        }
    }

    func testAuthorizationDecodeReadsAWellFormedRefusal() {
        let decision = BrowserFFIDecode.authorization(
            #"{"allowed":false,"opaque_write":false,"code":"tab_protected","message":"nope"}"#,
        )
        XCTAssertFalse(decision.allowed)
        XCTAssertEqual(decision.code, "tab_protected")
        XCTAssertFalse(decision.opaqueWrite)
    }

    func testAMissingOpaqueWriteFlagReadsAsReplaceTheResult() {
        // Restrictive direction, matching the Rust unknown-mode rule.
        let decision = BrowserFFIDecode.authorization(#"{"allowed":true}"#)
        XCTAssertTrue(decision.allowed)
        XCTAssertTrue(decision.opaqueWrite)
    }
}

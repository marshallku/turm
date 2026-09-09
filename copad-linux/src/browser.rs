//! The Linux half of the Browser Workbench dispatcher gate (work unit B1c).
//!
//! The rule itself is not here. `copad_core::browser::authorize` decides, and
//! this module only resolves the target and applies the decision — which is the
//! entire point: a second implementation of a security rule is a second
//! implementation that can be wrong on its own. See
//! `docs/browser-workbench-plan.md` §3.2 and decision #100.
//!
//! Three properties this module exists to uphold, each of which was a review
//! finding on the macOS side before it was a rule here:
//!
//! - **The gate runs before `ActionRegistry::try_dispatch`**, not before the
//!   legacy match. A browser method registered as an action would otherwise
//!   route around the gate entirely.
//! - **Authorization is re-checked at delivery**, not only at dispatch. Every
//!   value-returning browser method answers from a WebKit callback, so a read
//!   that was in flight when the tab entered [`TabMode::Protected`] must be
//!   suppressed rather than answered with a stale value.
//! - **A permitted write's result is replaced while protection is on.** `click`
//!   reporting ok-vs-not-found is a selector oracle. `copad_core` owns both the
//!   predicate ([`browser::authorize::redacts_write_result`]) and the replacement, so the
//!   redaction cannot drift between platforms — in particular it applies *only*
//!   under protection, leaving an unprotected pane's responses untouched.
//!
//! Target state lives on the [`WebViewPanel`] itself rather than in a parallel
//! registry. A registry would need unregistering on every pane-close path, and a
//! missed one leaves an entry that outlives its pane — exactly the stale-read
//! leak the delivery check exists to prevent. Reading through
//! [`TabManager::find_panel_by_id`] instead makes "the pane is gone" and "the
//! entry is gone" the same fact.

use std::rc::Rc;

use copad_core::browser::{self, TabMode};
use copad_core::protocol::{Request, Response, ResponseError};

use crate::tabs::TabManager;

/// Platform name in `unsupported_capability` messages.
const PLATFORM: &str = "Linux";

/// Browser methods that are WIRED but not built on Linux yet.
///
/// Registering them matters: a `coctl` script gets a truthful, machine-readable
/// "not implemented yet" instead of `unknown_method`, which is
/// indistinguishable from a typo — and the Linux port becomes leaf-filling
/// rather than surface design. Each name is removed from this list by the work
/// unit that implements it (B7a–B7f).
///
/// Every name here must also be in `copad_daemon::socket::BROWSER_RESERVED_METHODS`,
/// or a service plugin can claim it in `provides[]` and intercept the
/// daemon-routed call before it ever reaches this gate. The two lists are held
/// together by a test rather than by discipline.
pub const UNIMPLEMENTED: &[&str] = &[
    "webview.tab.new",
    "webview.tab.list",
    "webview.tab.select",
    "webview.tab.close",
    "webview.tab.move",
    "webview.tab.protect",
    "webview.profile.list",
    "webview.profile.clear",
    "browser.secret.list",
    "browser.secret.fill",
    "browser.secret.save",
    "browser.secret.delete",
    "webview.net",
    "webview.console",
    "webview.clear_log",
];

/// What [`gate`] decided about a request.
pub enum Gate {
    /// Not a browser method — the dispatcher proceeds untouched.
    NotBrowser,
    Refused(ResponseError),
    Allowed(GateCtx),
}

/// What the delivery-time re-check needs to ask the same question again.
///
/// Deliberately holds ids, not resolved state: replaying a mode captured at
/// dispatch is the bug the second check exists to catch.
#[derive(Clone)]
pub struct GateCtx {
    pub method: String,
    /// `None` for `webview.open`, which creates its own target.
    pub panel_id: Option<String>,
    /// Did the target resolve to a live webview when the request was admitted?
    ///
    /// This is what separates "the pane vanished mid-flight" from "there was
    /// never such a pane". [`finalize`] fails closed only for the first: a
    /// request that never had a target read nothing, and overwriting its
    /// handler's `not_found` with `tab_closed` would report a pane closing that
    /// never existed.
    resolved_at_dispatch: bool,
}

/// The target's live state, re-read on every check.
struct Resolved {
    mode: TabMode,
    profile_protected: bool,
}

/// True for names this gate is responsible for. `browser.*` vs `webview.*` is
/// naming, not a boundary (the boundary is [`TabMode`]); both are covered.
fn is_browser_method(method: &str) -> bool {
    method.starts_with("webview.") || method.starts_with("browser.")
}

/// Resolve a request's target against live panel state.
///
/// `Ok(None)` means "no resolvable webview target" — a bad panel id, or a panel
/// that is not a webview. At dispatch that is left to the existing handler,
/// which already answers `not_found` / `wrong_panel_type` and whose error text
/// callers depend on; there is nothing to protect in either case. At delivery it
/// is treated as fail-closed by [`finalize`], because a pane that vanished
/// mid-flight must not hand back what it read.
fn resolve(
    mgr: &Rc<TabManager>,
    panel_id: Option<&str>,
    tab_id: Option<&str>,
) -> Result<Option<Resolved>, ResponseError> {
    let profile_protected = |profile: &str| mgr.any_protected_webview_in_profile(profile);

    let Some(panel_id) = panel_id else {
        // `webview.open` creates its target, so it is judged against the
        // profile new panes are born into, in `Automation` mode.
        let profile = mgr.default_browser_profile();
        return Ok(Some(Resolved {
            mode: TabMode::Automation,
            profile_protected: profile_protected(&profile),
        }));
    };

    let Some(panel) = mgr.find_panel_by_id(panel_id) else {
        return Ok(None);
    };
    let Some(wv) = panel.as_webview() else {
        return Ok(None);
    };

    // A `tab_id` naming a tab this pane does not hold is `tab_closed`, never a
    // silent retarget onto whatever tab happens to be active (plan §3.2).
    if let Some(requested) = tab_id
        && requested != wv.tab_id
    {
        return Err(ResponseError {
            code: "tab_closed".into(),
            message: format!("tab {requested} is not open in panel {panel_id}"),
        });
    }

    Ok(Some(Resolved {
        mode: wv.mode(),
        profile_protected: profile_protected(&wv.profile),
    }))
}

/// Decide whether a request may run, before the action registry sees it.
///
/// Check order is load-bearing. "Not built yet" is answered BEFORE the policy
/// gate, because `browser.secret.fill` on an unprotected tab answers
/// `requires_protected` and sends the caller to `webview.tab.protect` — which is
/// itself unimplemented here. A truthful dead end beats a loop.
pub fn gate(req: &Request, mgr: &Rc<TabManager>) -> Gate {
    if !is_browser_method(&req.method) {
        return Gate::NotBrowser;
    }

    if UNIMPLEMENTED.contains(&req.method.as_str()) {
        return Gate::Refused(browser::authorize::unsupported_capability(
            &req.method,
            PLATFORM,
        ));
    }

    // Only methods core knows about are gated. An unrecognised `webview.*` name
    // falls through to the dispatcher's own unknown-method handling, so a typo
    // still reads as a typo.
    if browser::classify(&req.method).is_none() {
        return Gate::NotBrowser;
    }

    let panel_id = req.params.get("id").and_then(|v| v.as_str());
    let tab_id = req.params.get("tab_id").and_then(|v| v.as_str());

    let resolved = match resolve(mgr, panel_id, tab_id) {
        Ok(r) => r,
        Err(err) => return Gate::Refused(err),
    };

    if let Some(r) = &resolved
        && let Err(err) = browser::authorize(&req.method, r.mode, r.profile_protected)
    {
        return Gate::Refused(err);
    }

    Gate::Allowed(GateCtx {
        method: req.method.clone(),
        panel_id: panel_id.map(str::to_string),
        resolved_at_dispatch: resolved.is_some(),
    })
}

/// Apply the gate to a response on its way out.
///
/// Runs for both the synchronous arms and the WebKit-callback arms, because a
/// write is as capable of leaking through its error as a read is through its
/// result.
pub fn finalize(mgr: &Rc<TabManager>, ctx: &GateCtx, resp: Response) -> Response {
    let id = resp.id.clone();

    let resolved = match resolve(mgr, ctx.panel_id.as_deref(), None) {
        Ok(Some(r)) => r,
        // Fail closed, but only for a target that WAS live at dispatch: its mode
        // can no longer be established, so a result computed against it must not
        // be delivered. A request that never resolved is left to the handler's
        // own `not_found` / `wrong_panel_type`, which is the truthful answer.
        Ok(None) => {
            if ctx.resolved_at_dispatch {
                return Response::error(
                    id,
                    "tab_closed",
                    "the target pane closed before the result was delivered",
                );
            }
            return resp;
        }
        Err(err) => return Response::error(id, &err.code, &err.message),
    };

    if let Err(err) = browser::authorize(&ctx.method, resolved.mode, resolved.profile_protected) {
        return Response::error(id, &err.code, &err.message);
    }

    if browser::authorize::redacts_write_result(
        &ctx.method,
        resolved.mode,
        resolved.profile_protected,
    ) {
        return Response::success(id, browser::authorize::opaque_write_response());
    }

    resp
}

/// A reply path for the callback-deferred handlers, which move `cmd.reply` into
/// a closure and can no longer reach [`SocketCommand::reply_with_completion`].
///
/// Every such handler used to end with the same publish-then-send pair; routing
/// them through here is what makes the delivery-time re-check unskippable rather
/// than something each handler has to remember.
pub struct BrowserReply {
    reply: std::sync::mpsc::Sender<Response>,
    bus: copad_daemon::socket::EventBus,
    silent: bool,
    ctx: GateCtx,
    mgr: Rc<TabManager>,
}

impl BrowserReply {
    pub fn new(
        reply: std::sync::mpsc::Sender<Response>,
        bus: copad_daemon::socket::EventBus,
        silent: bool,
        ctx: GateCtx,
        mgr: Rc<TabManager>,
    ) -> Self {
        Self {
            reply,
            bus,
            silent,
            ctx,
            mgr,
        }
    }

    pub fn send(self, resp: Response) {
        let resp = finalize(&self.mgr, &self.ctx, resp);
        copad_daemon::socket::publish_legacy_completion(
            &self.bus,
            &self.ctx.method,
            self.silent,
            &resp,
        );
        let _ = self.reply.send(resp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These cover the parts that are pure decision, not GTK state: the check
    // ORDER and the method-name partition. Target resolution and the delivery
    // re-check need a live `TabManager`, so they are asserted by
    // `scripts/e2e-browser-linux-gate.sh` against the real socket instead.

    #[test]
    fn every_unimplemented_name_is_one_core_classifies() {
        // A name in this list that core does not know is a typo that would
        // answer `unsupported_capability` forever while the real method fell
        // through to `unknown_method`.
        for m in UNIMPLEMENTED {
            assert!(
                browser::classify(m).is_some(),
                "{m} is not a method copad-core classifies"
            );
        }
    }

    /// Every `webview.*` method the dispatcher has a match arm for.
    ///
    /// Each of those arms reads `browser_ctx` with `.expect()`, which is sound
    /// only while `gate` classifies the method — an unclassified one returns
    /// `NotBrowser`, leaves `browser_ctx` as `None`, and PANICS the GUI instead
    /// of answering. Adding an arm without adding the name to
    /// `copad_core::browser::classify` is therefore a crash, and this list is
    /// what turns it into a failing test.
    const DISPATCHED_ON_LINUX: &[&str] = &[
        "webview.open",
        "webview.navigate",
        "webview.back",
        "webview.forward",
        "webview.reload",
        "webview.execute_js",
        "webview.get_content",
        "webview.screenshot",
        "webview.query",
        "webview.query_all",
        "webview.get_styles",
        "webview.click",
        "webview.fill",
        "webview.scroll",
        "webview.page_info",
        "webview.state",
        "webview.devtools",
    ];

    #[test]
    fn every_registered_stub_is_reserved_against_plugin_shadowing() {
        // Daemon dispatch checks the action registry BEFORE it routes by
        // capability, so a service plugin listing `browser.secret.fill` in
        // `provides[]` would answer the call itself and the gate would never
        // run. Registering a method without reserving it opens exactly that
        // window, and the window lasts until the method is implemented.
        for m in UNIMPLEMENTED {
            assert!(
                copad_daemon::socket::BROWSER_RESERVED_METHODS.contains(m),
                "{m} is registered on Linux but not reserved against plugin `provides[]`"
            );
        }
    }

    #[test]
    fn a_dispatched_method_is_reserved_too() {
        // The built ones are covered by the older `LEGACY_DISPATCH_METHODS`
        // list, which predates this unit; asserted here so the two reservation
        // lists together cover the whole browser surface.
        for m in DISPATCHED_ON_LINUX {
            assert!(
                copad_daemon::socket::LEGACY_DISPATCH_METHODS.contains(m)
                    || copad_daemon::socket::BROWSER_RESERVED_METHODS.contains(m),
                "{m} is dispatched but not reserved against plugin `provides[]`"
            );
        }
    }

    #[test]
    fn every_dispatched_method_is_classified_so_the_arms_cannot_panic() {
        for m in DISPATCHED_ON_LINUX {
            assert!(
                browser::classify(m).is_some(),
                "{m} has a dispatcher arm but copad-core does not classify it — \
                 `browser_ctx.expect()` in that arm would panic the GUI"
            );
        }
    }

    #[test]
    fn a_dispatched_method_is_never_also_registered_as_unimplemented() {
        // The two lists partition the surface. An overlap answers
        // `unsupported_capability` for a method that is right there in the
        // match, which reads as a regression with no failing test behind it.
        for m in DISPATCHED_ON_LINUX {
            assert!(
                !UNIMPLEMENTED.contains(m),
                "{m} is both dispatched and stubbed"
            );
        }
    }

    #[test]
    fn implemented_read_and_write_methods_are_not_on_the_unimplemented_list() {
        for m in [
            "webview.execute_js",
            "webview.get_content",
            "webview.screenshot",
            "webview.query",
            "webview.query_all",
            "webview.get_styles",
            "webview.page_info",
            "webview.scroll",
            "webview.state",
            "webview.navigate",
            "webview.back",
            "webview.forward",
            "webview.reload",
            "webview.click",
            "webview.fill",
            "webview.open",
            "webview.devtools",
        ] {
            assert!(!UNIMPLEMENTED.contains(&m), "{m} is implemented on Linux");
        }
    }

    #[test]
    fn browser_namespace_is_covered_alongside_webview() {
        assert!(is_browser_method("webview.click"));
        assert!(is_browser_method("browser.secret.fill"));
        assert!(!is_browser_method("tab.switch"));
        assert!(!is_browser_method("terminal.read"));
    }

    #[test]
    fn an_unclassified_webview_name_is_left_to_the_unknown_method_path() {
        // A typo must stay a typo. `classify` returning `None` is what keeps a
        // forgotten entry from silently becoming an unguarded read, so the gate
        // must not invent a decision for it either.
        assert!(browser::classify("webview.opne").is_none());
        assert!(!UNIMPLEMENTED.contains(&"webview.opne"));
    }
}

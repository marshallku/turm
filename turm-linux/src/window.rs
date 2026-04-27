use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};
use serde_json::json;

use turm_core::action_registry::{ActionRegistry, internal_error};
use turm_core::config::TurmConfig;
use turm_core::context::ContextService;
use turm_core::event_bus::{Event as BusEvent, EventBus as CoreEventBus, EventReceiver};
use turm_core::trigger::{Trigger, TriggerEngine, TriggerSink, covering_patterns};

use crate::service_supervisor::ServiceSupervisor;
use crate::trigger_sink::LiveTriggerSink;

use crate::panel::Panel;
use crate::socket;
use crate::statusbar::StatusBar;
use crate::tabs::TabManager;

pub struct TurmWindow {
    pub window: ApplicationWindow,
    pub tab_manager: Rc<TabManager>,
    #[allow(dead_code)]
    statusbar: Rc<StatusBar>,
    /// `connect_destroy` calls `service_supervisor.shutdown_all()` to
    /// tear down service plugins (send the documented `shutdown`
    /// notification, drop writer-channel sender, SIGKILL stragglers
    /// after a grace window). Storing the supervisor on this struct
    /// also keeps the `Arc` count >= 1 for the duration of the
    /// window's lifetime so runtime threads aren't suddenly orphaned
    /// by a refcount drop.
    #[allow(dead_code)]
    service_supervisor: Arc<ServiceSupervisor>,
}

impl TurmWindow {
    pub fn new(app: &Application, config: &TurmConfig) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("turm")
            .default_width(1200)
            .default_height(800)
            .build();

        let theme = turm_core::theme::Theme::by_name(&config.theme.name).unwrap_or_default();
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(&format!(
            "window {{ background-color: {}; }}",
            theme.background
        ));
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let event_bus = socket::new_event_bus();

        // Context Service: live snapshot of "what the user is currently doing."
        // Pumped from the GTK timer below; exposed via the `context.snapshot` action.
        let context = Arc::new(ContextService::new());

        // Action Registry: shared across socket + plugin dispatch paths.
        // Migrating commands one at a time from the match arm in socket::dispatch.
        // `with_completion_bus` opts the registry into Phase 14.1 — every
        // dispatched action auto-publishes `<name>.completed` / `.failed`
        // on the bus so chained triggers compose without each plugin
        // having to emit completion events manually.
        let actions = Arc::new(ActionRegistry::with_completion_bus(event_bus.clone()));
        // High-frequency built-ins are registered "silent" so their
        // completions don't dwarf real workflow events on the bus.
        // system.ping fires from heartbeat probes; context.snapshot
        // fires from anything that wants to see the active panel
        // (potentially every keystroke in agent flows).
        actions.register_silent("system.ping", |_| Ok(json!({ "status": "ok" })));
        actions.register("system.log", |params| {
            // Built-in observable action — useful as a trigger sink. Falls
            // back to the full params JSON when no `message` field is present
            // so the user always sees something even with a misshapen call.
            let msg = params
                .get("message")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| params.to_string());
            eprintln!("[system.log] {msg}");
            Ok(json!({}))
        });
        {
            let ctx = context.clone();
            actions.register_silent("context.snapshot", move |_| {
                serde_json::to_value(ctx.snapshot())
                    .map_err(|e| internal_error(format!("snapshot serialization failed: {e}")))
            });
        }

        // Dispatch channel: TabManager owns the original sender (used by
        // plugin JS bridges). Trigger sink gets a clone so trigger-fired
        // legacy actions (anything not in the registry yet) can fall through
        // to `socket::dispatch` via the same path.
        let (dispatch_tx, plugin_dispatch_rx) = std::sync::mpsc::channel();

        // Trigger Engine + scoped bus subscriptions. Sink is `LiveTriggerSink`,
        // which tries the in-process registry first and falls through to
        // `socket::dispatch` for legacy match-arm commands. That makes every
        // existing action (`tab.*`, `terminal.exec`, `webview.*`, `plugin.*`,
        // …) reachable from triggers without per-command migration.
        // Fire-and-forget caveat for fallthrough actions is documented on
        // `LiveTriggerSink` itself.
        //
        // PumpState bundles every per-tick drain target — context-driving
        // receivers AND trigger subscriptions — so the timer and the config
        // hot-reload callback can both invoke the same `pump_all` sequence.
        // Exact-match context subscriptions (not `*` and not glob) so high-
        // frequency unrelated kinds cannot flood the bounded ctx queues.
        let sink: Arc<dyn TriggerSink> =
            Arc::new(LiveTriggerSink::new(actions.clone(), dispatch_tx.clone()));
        let triggers = Arc::new(TriggerEngine::new(sink));
        triggers.set_triggers(config.triggers.clone());
        let pump_state = Rc::new(RefCell::new(PumpState {
            ctx_focused: event_bus.subscribe("panel.focused"),
            ctx_exited: event_bus.subscribe("panel.exited"),
            ctx_cwd: event_bus.subscribe("terminal.cwd_changed"),
            trigger_subs: TriggerSubscriptions::new(),
        }));
        pump_state
            .borrow_mut()
            .reconcile_triggers(&event_bus, &config.triggers);
        eprintln!(
            "[turm] trigger engine: {} configured ({:?}) | {} bus pattern(s) subscribed",
            triggers.count(),
            triggers.names(),
            pump_state.borrow().trigger_subs_len()
        );

        // Plugin discovery
        let plugins = turm_core::plugin::discover_plugins();
        for p in &plugins {
            eprintln!(
                "[turm] plugin loaded: {} v{}",
                p.manifest.plugin.name, p.manifest.plugin.version
            );
        }

        // Service plugins: spawn long-running supervised subprocesses for
        // every `[[services]]` declaration. The supervisor walks every
        // manifest to resolve `provides` conflicts BEFORE spawning so
        // ownership stays deterministic (lexical name wins). Built-ins
        // already in the registry (system.ping, system.log,
        // context.snapshot) are reserved against plugin override.
        // Approved actions register through the same registry as
        // built-ins, so socket dispatch and triggers reach service
        // plugins identically. The Arc is stored on the window struct
        // so the lifetime is explicit.
        let service_supervisor = ServiceSupervisor::new(
            event_bus.clone(),
            actions.clone(),
            &plugins,
            env!("CARGO_PKG_VERSION"),
            socket::LEGACY_DISPATCH_METHODS,
        );

        // Socket server (per-instance, so multiple turm windows don't collide)
        let socket_path = format!("/tmp/turm-{}.sock", std::process::id());
        let socket_rx = socket::start_server(&socket_path, event_bus.clone());

        let tab_manager = TabManager::new(
            config,
            &window,
            event_bus.clone(),
            plugins.clone(),
            dispatch_tx,
        );

        // Status bar
        let statusbar = Rc::new(StatusBar::new(config, &plugins));

        // Layout: vertical box with notebook + statusbar
        let layout = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        if config.statusbar.position == "top" {
            layout.append(&statusbar.container);
            layout.append(&tab_manager.notebook);
        } else {
            layout.append(&tab_manager.notebook);
            layout.append(&statusbar.container);
        }
        window.set_child(Some(&layout));

        // Config hot-reload (also reloads triggers atomically)
        watch_config(
            &tab_manager,
            &statusbar,
            &plugins,
            &triggers,
            &event_bus,
            &pump_state,
            &context,
        );

        let mgr = tab_manager.clone();
        let win = window.clone();
        let sp = socket_path.clone();
        let sb = statusbar.clone();
        let act = actions.clone();
        let ctx_pump = context.clone();
        let trg_pump = triggers.clone();
        let pump_state_timer = pump_state.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            // `pump_all` drains context-driving events first, then trigger
            // events with per-event context snapshot. Single helper used by
            // both this timer and the hot-reload callback so semantics match.
            pump_state_timer.borrow().pump_all(&ctx_pump, &trg_pump);

            // Process socket commands. After each, drain ONLY context
            // receivers (not trigger queues — those are handled at the start
            // and end of tick). A dispatched command can publish events
            // (`tab.new` → `panel.focused`) and the very next command in the
            // same batch (e.g. `context.snapshot`) must see those events
            // applied to ContextService.
            while let Ok(cmd) = socket_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb, &act);
                pump_state_timer.borrow().drain_context_only(&ctx_pump);
            }
            while let Ok(cmd) = plugin_dispatch_rx.try_recv() {
                socket::dispatch(cmd, &mgr, &win, &sp, &sb, &act);
                pump_state_timer.borrow().drain_context_only(&ctx_pump);
            }
            glib::ControlFlow::Continue
        });

        // Cleanup socket and tear down service plugins on window
        // destroy. `shutdown_all` sends the documented `shutdown`
        // notification, drops the writer-channel sender (so child
        // stdin closes on EOF), and SIGKILLs anything still alive
        // after a brief grace window — children don't outlive the GUI.
        let socket_path_cleanup = socket_path.clone();
        let supervisor_cleanup = service_supervisor.clone();
        window.connect_destroy(move |_| {
            supervisor_cleanup.shutdown_all();
            socket::cleanup(&socket_path_cleanup);
        });

        Self {
            window,
            tab_manager,
            statusbar,
            service_supervisor,
        }
    }

    pub fn present(&self) {
        self.window.present();
        // Focus the terminal after the window is mapped
        let mgr = self.tab_manager.clone();
        glib::idle_add_local_once(move || {
            if let Some(panel) = mgr.active_panel() {
                panel.grab_focus();
            }
        });
    }
}

fn watch_config(
    tab_manager: &Rc<TabManager>,
    statusbar: &Rc<StatusBar>,
    plugins: &[turm_core::plugin::LoadedPlugin],
    triggers: &Arc<TriggerEngine>,
    event_bus: &Arc<CoreEventBus>,
    pump_state: &Rc<RefCell<PumpState>>,
    context: &Arc<ContextService>,
) {
    let config_path = TurmConfig::config_path();
    let file = gio::File::for_path(&config_path);

    let monitor = match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[turm] failed to watch config: {}", e);
            return;
        }
    };

    let mgr = tab_manager.clone();
    let sb = statusbar.clone();
    let pl = plugins.to_vec();
    let trg = triggers.clone();
    let bus = event_bus.clone();
    let ps = pump_state.clone();
    let ctx = context.clone();
    monitor.connect_changed(move |_, _, _, event| {
        if !matches!(
            event,
            gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created
        ) {
            return;
        }

        let config = match TurmConfig::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[turm] config reload error: {}", e);
                return;
            }
        };

        eprintln!("[turm] config reloaded");
        mgr.update_config(&config);
        sb.reload(&config, &pl);
        // `set_triggers` swaps the list atomically so a concurrent dispatch
        // sees either the old full list or the new full list, never a mix.
        trg.set_triggers(config.triggers.clone());
        // Pump everything on the CURRENT receiver set against the freshly-
        // installed trigger list BEFORE reconcile potentially drops any
        // receivers. `pump_all` is the same helper the timer uses, so:
        //   1. context queues are drained → ContextService is up-to-date,
        //   2. then trigger queues are drained with that fresh context for
        //      `{context.*}` interpolation.
        // Without step 1, a pattern-changing reload could fire pending
        // triggers with stale `{context.*}` values; without the whole drain,
        // a pattern edit (e.g. `terminal.cwd_changed` → `terminal.*`) would
        // discard pending events the new trigger set would have matched.
        ps.borrow().pump_all(&ctx, &trg);
        // Reconcile bus subscriptions: keep covering patterns still in use,
        // drop those no trigger needs (queues guaranteed empty by the pump
        // above), add new ones.
        ps.borrow_mut().reconcile_triggers(&bus, &config.triggers);
        eprintln!(
            "[turm] triggers reloaded: {} active ({:?}) | {} bus pattern(s) subscribed",
            trg.count(),
            trg.names(),
            ps.borrow().trigger_subs_len()
        );
    });

    std::mem::forget(monitor);
}

/// Bundles all per-tick drain targets — context-driving receivers AND the
/// trigger engine's per-pattern subscriptions — so the GTK timer and the
/// hot-reload callback can both invoke the same `pump_all` sequence with
/// identical semantics. Without this, the two callsites had subtly
/// different ordering (timer drained context first via a free function;
/// reload drained only triggers), causing reload-time `{context.*}`
/// interpolation to read stale state.
pub struct PumpState {
    ctx_focused: EventReceiver,
    ctx_exited: EventReceiver,
    ctx_cwd: EventReceiver,
    trigger_subs: TriggerSubscriptions,
}

impl PumpState {
    /// Drain the three context receivers into the ContextService. Order
    /// across them is not significant: focused/exited and cwd_changed for
    /// different panels are commutative for context's state model.
    pub fn drain_context_only(&self, ctx: &ContextService) {
        while let Some(event) = self.ctx_focused.try_recv() {
            ctx.apply_event(&event);
        }
        while let Some(event) = self.ctx_exited.try_recv() {
            ctx.apply_event(&event);
        }
        while let Some(event) = self.ctx_cwd.try_recv() {
            ctx.apply_event(&event);
        }
    }

    /// Drain context first (so `{context.*}` interpolation is fresh), then
    /// drain trigger subscriptions and dispatch them. One context snapshot
    /// per dispatched event keeps each invocation self-consistent.
    pub fn pump_all(&self, ctx: &ContextService, engine: &TriggerEngine) {
        self.drain_context_only(ctx);
        self.trigger_subs.drain_into(|event| {
            let snap = ctx.snapshot();
            engine.dispatch(&event, Some(&snap));
        });
    }

    pub fn reconcile_triggers(&mut self, bus: &Arc<CoreEventBus>, triggers: &[Trigger]) {
        self.trigger_subs.reconcile(bus, triggers);
    }

    pub fn trigger_subs_len(&self) -> usize {
        self.trigger_subs.len()
    }
}

/// Holds one bus receiver per unique `event_kind` pattern across all
/// currently-active triggers. Reconciled at startup and on every hot reload
/// so patterns no longer in use are dropped (their queues GC'd lazily on
/// the next bus publish) while still-needed patterns retain their existing
/// receiver — pending events are not lost when a reload changes only
/// unrelated triggers.
pub struct TriggerSubscriptions {
    receivers: HashMap<String, EventReceiver>,
}

impl TriggerSubscriptions {
    pub fn new() -> Self {
        Self {
            receivers: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.receivers.len()
    }

    /// Bring the active set of subscriptions in line with the kinds declared
    /// by `triggers`. The set is reduced via `covering_patterns` first so
    /// overlapping declarations (e.g. `*` plus `panel.focused`) collapse to a
    /// single broader receiver — otherwise the bus would deliver the same
    /// event to multiple receivers and trigger every matching action once
    /// per delivery, double-firing real side effects.
    ///
    /// Existing receivers for patterns still in the covering set are kept
    /// (their pending events are preserved across hot reload); receivers for
    /// removed patterns are dropped (lazily GC'd by the bus on next publish);
    /// new patterns get fresh `subscribe_unbounded` receivers (lossless for
    /// matched kinds).
    pub fn reconcile(&mut self, bus: &Arc<CoreEventBus>, triggers: &[Trigger]) {
        let raw: Vec<String> = triggers.iter().map(|t| t.when.event_kind.clone()).collect();
        let needed: std::collections::HashSet<String> =
            covering_patterns(raw).into_iter().collect();
        self.receivers.retain(|pattern, _| needed.contains(pattern));
        for pattern in needed {
            self.receivers
                .entry(pattern.clone())
                .or_insert_with(|| bus.subscribe_unbounded(pattern.clone()));
        }
    }

    /// Drain every receiver fully, calling `f` for each event. Order across
    /// receivers is not significant for trigger semantics — every event is
    /// matched against the full trigger list.
    pub fn drain_into<F: FnMut(BusEvent)>(&self, mut f: F) {
        for rx in self.receivers.values() {
            while let Some(event) = rx.try_recv() {
                f(event);
            }
        }
    }
}

impl Default for TriggerSubscriptions {
    fn default() -> Self {
        Self::new()
    }
}

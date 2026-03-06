use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use custerm_core::background::BackgroundManager;

const BUS_NAME: &str = "com.marshall.custerm";
const OBJECT_PATH: &str = "/com/marshall/custerm";
const INTERFACE_NAME: &str = "com.marshall.custerm";

const INTROSPECTION_XML: &str = r#"
<node>
  <interface name="com.marshall.custerm">
    <method name="SetBackground">
      <arg name="path" type="s" direction="in"/>
    </method>
    <method name="NextBackground"/>
    <method name="ClearBackground"/>
    <method name="SetTint">
      <arg name="opacity" type="d" direction="in"/>
    </method>
    <method name="GetCurrentBackground">
      <arg name="path" type="s" direction="out"/>
    </method>
  </interface>
</node>
"#;

#[derive(Debug, Clone)]
pub enum DbusCommand {
    SetBackground(String),
    ClearBackground,
    SetTint(f64),
}

/// Register D-Bus service. Returns an mpsc::Receiver for commands.
/// Caller must poll the receiver on the GTK main thread.
pub fn register(bg_manager: Arc<Mutex<BackgroundManager>>) -> mpsc::Receiver<DbusCommand> {
    let (tx, rx) = mpsc::channel::<DbusCommand>();

    gio::bus_own_name(
        gio::BusType::Session,
        BUS_NAME,
        gio::BusNameOwnerFlags::NONE,
        {
            let tx = tx.clone();
            let bg_manager = bg_manager.clone();
            move |connection, _name| {
                let node_info = gio::DBusNodeInfo::for_xml(INTROSPECTION_XML)
                    .expect("Failed to parse introspection XML");
                let interface_info = node_info
                    .lookup_interface(INTERFACE_NAME)
                    .expect("Interface not found");

                let tx = tx.clone();
                let bg_manager = bg_manager.clone();
                let _reg_id = connection
                    .register_object(OBJECT_PATH, &interface_info)
                    .method_call(
                        move |_conn, _sender, _path, _interface, method, params, invocation| {
                            handle_method(&tx, &bg_manager, method, params, invocation);
                        },
                    )
                    .build();
            }
        },
        |_connection, _name| {},
        |_connection, _name| {
            log::warn!("Lost D-Bus name ownership");
        },
    );

    rx
}

fn handle_method(
    tx: &mpsc::Sender<DbusCommand>,
    bg_manager: &Arc<Mutex<BackgroundManager>>,
    method: &str,
    params: glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "SetBackground" => {
            let path: String = params.child_get(0);
            if let Ok(mut mgr) = bg_manager.lock() {
                mgr.current = Some(PathBuf::from(&path));
            }
            let _ = tx.send(DbusCommand::SetBackground(path));
            invocation.return_value(None);
        }
        "NextBackground" => {
            let path = {
                let mut mgr = bg_manager.lock().unwrap();
                mgr.next().map(|p| p.to_path_buf())
            };
            if let Some(path) = path {
                let _ = tx.send(DbusCommand::SetBackground(
                    path.to_string_lossy().to_string(),
                ));
            }
            invocation.return_value(None);
        }
        "ClearBackground" => {
            let _ = tx.send(DbusCommand::ClearBackground);
            invocation.return_value(None);
        }
        "SetTint" => {
            let opacity: f64 = params.child_get(0);
            let _ = tx.send(DbusCommand::SetTint(opacity));
            invocation.return_value(None);
        }
        "GetCurrentBackground" => {
            let path = bg_manager
                .lock()
                .ok()
                .and_then(|mgr| mgr.current.as_ref().map(|p| p.to_string_lossy().to_string()))
                .unwrap_or_default();
            invocation.return_value(Some(&(path,).to_variant()));
        }
        _ => {
            invocation.return_error(gio::IOErrorEnum::Failed, &format!("Unknown method: {method}"));
        }
    }
}

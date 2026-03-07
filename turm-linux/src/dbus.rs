use std::sync::mpsc;

use gtk4::gio;
use gtk4::glib;

const OBJECT_PATH: &str = "/com/marshall/turm";
const INTERFACE_NAME: &str = "com.marshall.turm";

const INTROSPECTION_XML: &str = r#"
<node>
  <interface name="com.marshall.turm">
    <method name="SetBackground">
      <arg name="path" type="s" direction="in"/>
    </method>
    <method name="ClearBackground"/>
    <method name="SetTint">
      <arg name="opacity" type="d" direction="in"/>
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

/// Per-process D-Bus name: com.marshall.turm.p{PID}
pub fn bus_name() -> String {
    format!("com.marshall.turm.p{}", std::process::id())
}

pub fn register() -> mpsc::Receiver<DbusCommand> {
    let (tx, rx) = mpsc::channel::<DbusCommand>();
    let name = bus_name();

    eprintln!("[turm] D-Bus name: {}", name);

    gio::bus_own_name(
        gio::BusType::Session,
        &name,
        gio::BusNameOwnerFlags::NONE,
        {
            let tx = tx.clone();
            move |connection, _name| {
                let node_info = gio::DBusNodeInfo::for_xml(INTROSPECTION_XML)
                    .expect("Failed to parse introspection XML");
                let interface_info = node_info
                    .lookup_interface(INTERFACE_NAME)
                    .expect("Interface not found");

                let tx = tx.clone();
                let _reg_id = connection
                    .register_object(OBJECT_PATH, &interface_info)
                    .method_call(
                        move |_conn, _sender, _path, _interface, method, params, invocation| {
                            handle_method(&tx, method, params, invocation);
                        },
                    )
                    .build();
            }
        },
        |_connection, _name| {},
        |_connection, _name| {
            eprintln!("[turm] lost D-Bus name ownership");
        },
    );

    rx
}

fn handle_method(
    tx: &mpsc::Sender<DbusCommand>,
    method: &str,
    params: glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "SetBackground" => {
            let path: String = params.child_get(0);
            let _ = tx.send(DbusCommand::SetBackground(path));
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
        _ => {
            invocation.return_error(
                gio::IOErrorEnum::Failed,
                &format!("Unknown method: {method}"),
            );
        }
    }
}

//! The tray on a real (private) D-Bus: a `dbus-daemon` of the test's own with a fake
//! StatusNotifierWatcher, never the user's session bus. The parent test starts the bus and runs
//! the child test in a process of its own with `DBUS_SESSION_BUS_ADDRESS` pointing at it (so no
//! environment is changed in this multi-threaded test process).

use super::*;
use std::collections::HashMap;
use std::io::BufRead;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const CHILD: &str = "tray::dbus_tests::tray_on_a_private_bus_child";
const BUS_ENV: &str = "LODESTAR_TEST_PRIVATE_BUS";
/// The child got to the end (not an early return).
const DONE: &str = "tray checks on the private bus done";

#[test]
fn tray_on_a_private_bus() {
    let Some(daemon) = crate::launch::find_in_path("dbus-daemon") else {
        eprintln!("dbus-daemon not available; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus");
    // No service dirs: nothing on this bus is ever auto-started.
    let conf = dir.path().join("bus.conf");
    std::fs::write(
        &conf,
        format!(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \
             \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
             <busconfig><type>session</type><listen>unix:path={}</listen>\
             <auth>EXTERNAL</auth>\
             <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
             <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n",
            sock.display()
        ),
    )
    .unwrap();
    let mut bus = std::process::Command::new(daemon)
        .arg(format!("--config-file={}", conf.display()))
        .args(["--nofork", "--print-address=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut addr = String::new();
    std::io::BufReader::new(bus.stdout.take().unwrap()).read_line(&mut addr).unwrap();
    let addr = addr.trim().to_string();
    assert!(addr.starts_with("unix:"), "{addr:?}");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", CHILD, "--nocapture", "--test-threads=1"])
        .env("DBUS_SESSION_BUS_ADDRESS", &addr)
        .env(BUS_ENV, &addr)
        .output()
        .unwrap();
    let _ = bus.kill();
    let _ = bus.wait();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.contains("1 passed") && text.contains(DONE), "{text}");
}

struct Watcher {
    items: std::sync::mpsc::Sender<String>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        let _ = self.items.send(service);
    }
    fn register_status_notifier_host(&self, _service: String) {}
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
/// StatusNotifierItem's ToolTip: (icon name, icon pixmaps, title, description).
type ToolTipValue = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

/// The menu as (id, label, enabled); separators as "--".
fn menu_items(layout: &Layout) -> Vec<(i32, String, bool)> {
    layout
        .2
        .iter()
        .map(|v| {
            let (id, props, _): Layout = v.try_clone().unwrap().try_into().unwrap();
            let s = |k: &str| props.get(k).and_then(|v| String::try_from(v.try_clone().unwrap()).ok());
            let label =
                if s("type").as_deref() == Some("separator") { "--".into() } else { s("label").unwrap_or_default() };
            let enabled = props.get("enabled").is_none_or(|v| bool::try_from(v.try_clone().unwrap()).unwrap_or(true));
            (id, label, enabled)
        })
        .collect()
}

#[test]
#[ignore = "run by tray_on_a_private_bus in a process of its own"]
fn tray_on_a_private_bus_child() {
    // Only ever on the private bus the parent started.
    let Some(addr) = std::env::var(BUS_ENV).ok().filter(|a| !a.is_empty()) else { return };
    assert_eq!(std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(), Some(addr.as_str()));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    // Without a watcher: no icon (yet).
    let ctx = egui::Context::default();
    let (late_tx, late_rx) = crossbeam_channel::unbounded();
    let late = start(&ctx, late_tx, TrayState::default());
    assert_eq!(late_rx.recv_timeout(Duration::from_secs(10)).unwrap(), TrayEvent::Host(false));

    let (items, stop_watcher) = fake_watcher(&addr);

    // The watcher appearing later shows the icon after all.
    assert_eq!(late_rx.recv_timeout(Duration::from_secs(10)).unwrap(), TrayEvent::Host(true));
    let late_service = items.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(late);
    gone_from_the_bus(&rt, &addr, &late_service);

    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = start(&ctx, tx, TrayState { shown: true, game: None, can_stop: true });
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), TrayEvent::Host(true));
    let service = items.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(service.starts_with("org.kde.StatusNotifierItem-"), "{service}");

    rt.block_on(async {
        let c = zbus::connection::Builder::address(addr.as_str()).unwrap().build().await.unwrap();
        let proxy = |path: String, iface: &'static str| {
            let c = c.clone();
            let service = service.clone();
            async move {
                zbus::proxy::Builder::<zbus::Proxy>::new(&c)
                    .destination(service)
                    .unwrap()
                    .path(path)
                    .unwrap()
                    .interface(iface)
                    .unwrap()
                    .cache_properties(zbus::proxy::CacheProperties::No)
                    .build()
                    .await
                    .unwrap()
            }
        };
        let item = proxy("/StatusNotifierItem".into(), "org.kde.StatusNotifierItem").await;
        let title: String = item.get_property("Title").await.unwrap();
        assert_eq!(title, "Lodestar");
        let pix: Vec<(i32, i32, Vec<u8>)> = item.get_property("IconPixmap").await.unwrap();
        assert_eq!(pix.iter().map(|p| p.0).collect::<Vec<_>>(), [16, 22, 24, 32, 48, 64, 128]);
        assert!(pix.iter().all(|(w, h, d)| d.len() == (w * h * 4) as usize));
        let tip: ToolTipValue = item.get_property("ToolTip").await.unwrap();
        assert_eq!(tip.2, "Lodestar");

        // Left click toggles.
        let () = item.call("Activate", &(0i32, 0i32)).await.unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), TrayEvent::Toggle);

        let menu_path: OwnedObjectPath = item.get_property("Menu").await.unwrap();
        let menu = proxy(menu_path.to_string(), "com.canonical.dbusmenu").await;
        let get = || async {
            let (_rev, layout): (u32, Layout) =
                menu.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).await.unwrap();
            menu_items(&layout)
        };
        let m = get().await;
        let labels: Vec<&str> = m.iter().map(|i| i.1.as_str()).collect();
        assert_eq!(labels, ["Hide Lodestar", "--", "Quit Lodestar"]);
        let () = menu.call("Event", &(m[2].0, "clicked", Value::from(0i32), 0u32)).await.unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), TrayEvent::Quit);

        // A game runs and the window is hidden: the menu and tooltip follow.
        handle.set_state(TrayState { shown: false, game: Some("Hades_II".into()), can_stop: true });
        let mut m = vec![];
        for _ in 0..50 {
            m = get().await;
            if m.len() == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let labels: Vec<(&str, bool)> = m.iter().map(|i| (i.1.as_str(), i.2)).collect();
        // Underscores are doubled: dbusmenu would read one as an access key.
        assert_eq!(labels, [("Show Lodestar", true), ("Stop Hades__II", true), ("--", true), ("Quit Lodestar", true)]);
        let tip: ToolTipValue = item.get_property("ToolTip").await.unwrap();
        assert_eq!(tip.2, "Lodestar — playing Hades_II");
        let () = menu.call("Event", &(m[1].0, "clicked", Value::from(0i32), 0u32)).await.unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), TrayEvent::StopGame);
        let () = menu.call("Event", &(m[0].0, "clicked", Value::from(0i32), 0u32)).await.unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), TrayEvent::Show);
    });

    // The tray goes away (Plasma restarting): the app hears of it (and shows a hidden window);
    // when it is back, so is the icon.
    stop_watcher.store(true, Ordering::SeqCst);
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), TrayEvent::Host(false));
    let (items, _stop) = fake_watcher(&addr);
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), TrayEvent::Host(true));
    assert_eq!(items.recv_timeout(Duration::from_secs(10)).unwrap(), service, "registered again");

    // Dropping the handle removes the icon (the name goes away), and says nothing more.
    drop(handle);
    gone_from_the_bus(&rt, &addr, &service);
    assert!(rx.try_recv().is_err(), "nothing after the handle is dropped");
    eprintln!("{DONE}");
}

/// A fake StatusNotifierWatcher on the bus, on a thread that drives its own runtime: the items
/// that register, and a flag that takes it off the bus again.
fn fake_watcher(addr: &str) -> (std::sync::mpsc::Receiver<String>, Arc<AtomicBool>) {
    let (items_tx, items) = std::sync::mpsc::channel();
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let (a, st) = (addr.to_string(), stop.clone());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let conn = zbus::connection::Builder::address(a.as_str())
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at("/StatusNotifierWatcher", Watcher { items: items_tx })
                .unwrap()
                .build()
                .await
                .unwrap();
            ready_tx.send(()).unwrap();
            while !st.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let _ = conn.close().await;
        });
    });
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    (items, stop)
}

fn gone_from_the_bus(rt: &tokio::runtime::Runtime, addr: &str, service: &str) {
    rt.block_on(async {
        let c = zbus::connection::Builder::address(addr).unwrap().build().await.unwrap();
        let dbus = zbus::fdo::DBusProxy::new(&c).await.unwrap();
        let name = zbus::names::BusName::try_from(service).unwrap();
        for _ in 0..50 {
            if !dbus.name_has_owner(name.clone()).await.unwrap() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("{service} still on the bus");
    });
}

//! rustmesh Mesh Studio: 2D and 3D meshing on desktop and in the browser.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use rustmesh_app::app;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("--mesh-worker") {
        if let Err(e) = rustmesh_app::jobs::worker_main() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("rustmesh — Mesh Studio")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rustmesh",
        options,
        Box::new(|cc| Ok(Box::new(app::MeshApp::new(cc)))),
    )
}

/// Runs on loading the WebAssembly module, in the page and in the meshing
/// worker alike; only the page has a window to draw in.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let canvas = document
        .get_element_by_id("rustmesh")
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("the page has a canvas with id \"rustmesh\"");
    wasm_bindgen_futures::spawn_local(async move {
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(app::MeshApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(e) => loading
                    .set_text_content(Some(&format!("The mesh builder could not start: {e:?}"))),
            }
        }
    });
}

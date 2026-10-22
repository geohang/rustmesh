//! Meshing in an isolated native process or a terminable browser Worker.
pub use rustmesh::control::{Limits, StopReason};
use rustmesh::{Geometry2D, Mesh2D, Mesh3D, Options3D, Plc3D};
use serde::{Deserialize, Serialize};
#[path = "jobs_contract.rs"]
mod contract;
pub use contract::{GeometryStatus, GoalStatus, MeshOutcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "dimension")]
pub enum Request {
    TwoD {
        geometry: Geometry2D,
        max_nodes: usize,
    },
    ThreeD {
        plc: Plc3D,
        options: Options3D,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "dimension", content = "mesh")]
pub enum Product {
    TwoD(Mesh2D),
    ThreeD(Mesh3D),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub mesh: Result<Product, String>,
    pub seconds: f64,
    #[serde(default)]
    pub status: MeshOutcome,
}
impl Outcome {
    pub fn failure(reason: StopReason, message: impl Into<String>, seconds: f64) -> Self {
        Self {
            mesh: Err(message.into()),
            seconds,
            status: MeshOutcome {
                stop_reason: reason,
                ..Default::default()
            },
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn run(request: Request, now: impl Fn() -> f64 + Send + Sync + 'static) -> Outcome {
    let limits = Limits {
        max_nodes: match &request {
            Request::TwoD { max_nodes, .. } => *max_nodes,
            Request::ThreeD { options, .. } => options.max_vertices,
        },
        ..Limits::default()
    };
    run_with_limits(request, limits, now)
}
pub fn run_with_limits(
    mut request: Request,
    limits: Limits,
    now: impl Fn() -> f64 + Send + Sync + 'static,
) -> Outcome {
    use sha2::{Digest, Sha256};
    if let Err(e) = limits.validate() {
        return Outcome::failure(StopReason::BackendFailure, e.to_string(), 0.0);
    }
    match &mut request {
        Request::TwoD { max_nodes, .. } => *max_nodes = (*max_nodes).min(limits.max_nodes),
        Request::ThreeD { options, .. } => {
            options.max_vertices = options.max_vertices.min(limits.max_nodes)
        }
    }
    let start = now();
    let clock = std::sync::Arc::new(now);
    let c = clock.clone();
    let control = rustmesh::control::MeshControl::new(limits.clone(), move || c() - start);
    let mut result = rustmesh::control::with_control(control.clone(), || match &request {
        Request::TwoD {
            geometry,
            max_nodes,
        } => geometry.mesh(*max_nodes).map(Product::TwoD),
        Request::ThreeD { plc, options } => {
            rustmesh::tetrahedralize(plc, options).map(Product::ThreeD)
        }
    });
    if let Some(reason) = control.stop_reason() {
        result = Err(rustmesh::MeshError::Stopped(reason));
    }
    let reason = match &result {
        Err(rustmesh::MeshError::Stopped(r)) => *r,
        Err(rustmesh::MeshError::Insertion(e)) if e.contains("recovery") => {
            StopReason::GeometryRecoveryFailed
        }
        Err(rustmesh::MeshError::Insertion(e)) if e.contains("pass budget") => {
            StopReason::PassBudget
        }
        _ => StopReason::BackendFailure,
    };
    let seconds = clock() - start;
    let mut outcome = Outcome {
        mesh: result.map_err(|e| e.to_string()),
        seconds,
        status: MeshOutcome {
            stop_reason: reason,
            ..Default::default()
        },
    };
    contract::assess(&request, &mut outcome);
    outcome.status.input_hash = serde_json::to_vec(&request)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    outcome.status.effective_options = serde_json::json!({"request":request,"limits":limits});
    outcome.status.resource_usage = serde_json::json!({"wall_seconds":seconds,"iterations":control.iterations(),"memory_enforcement":"unsupported","cell_budget_scope":"working_complex_including_exterior"});
    outcome
}
#[cfg(not(target_arch = "wasm32"))]
#[path = "jobs_native.rs"]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{worker_main, Job, ProcessJob};
#[cfg(target_arch = "wasm32")]
pub use web::Job;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::prelude::*;
    use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

    use eframe::egui;

    use super::{run, Limits, Outcome, Request, StopReason};

    /// What the worker (`web/worker.js`) calls: a JSON [`Request`] in, a
    /// JSON [`Outcome`] out.
    #[wasm_bindgen]
    pub fn mesh_job(request: &str) -> String {
        let outcome = match serde_json::from_str::<Request>(request) {
            Ok(request) => run(request, || js_sys::Date::now() / 1000.0),
            Err(e) => Outcome {
                mesh: Err(format!("the worker got a bad request: {e}")),
                seconds: 0.0,
                status: Default::default(),
            },
        };
        let encoded = serde_json::to_string(&outcome).unwrap_or_else(|e| {
            format!(r#"{{"mesh":{{"Err":"could not send the mesh back: {e}"}},"seconds":0}}"#)
        });
        if encoded.len() as u64 > Limits::default().max_output_bytes {
            serde_json::to_string(&Outcome::failure(
                StopReason::OutputBudget,
                "serialized mesh exceeded output budget",
                outcome.seconds,
            ))
            .unwrap()
        } else {
            encoded
        }
    }

    pub struct Job {
        worker: Worker,
        slot: Rc<RefCell<Option<Outcome>>>,
        _on_message: Closure<dyn FnMut(MessageEvent)>,
        _on_error: Closure<dyn FnMut(ErrorEvent)>,
        _on_timeout: Closure<dyn FnMut()>,
        timeout_id: i32,
    }

    impl Job {
        pub fn start(ctx: &egui::Context, request: Request) -> Result<Job, String> {
            let options = WorkerOptions::new();
            options.set_type(WorkerType::Module);
            let worker = Worker::new_with_options("./worker.js", &options)
                .map_err(|e| format!("the browser could not start a worker: {e:?}"))?;
            let slot = Rc::new(RefCell::new(None));

            let (s, c) = (slot.clone(), ctx.clone());
            let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let text = event.data().as_string().unwrap_or_default();
                let outcome = serde_json::from_str(&text).unwrap_or_else(|e| Outcome {
                    mesh: Err(format!("the worker sent back something unreadable: {e}")),
                    seconds: 0.0,
                    status: Default::default(),
                });
                *s.borrow_mut() = Some(outcome);
                c.request_repaint();
            });
            let (s, c) = (slot.clone(), ctx.clone());
            let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
                event.prevent_default();
                *s.borrow_mut() = Some(Outcome {
                    mesh: Err(format!(
                        "meshing stopped in the browser ({}); the model may be too large for the web app, \
                         try the desktop app",
                        event.message()
                    )),
                    seconds: 0.0, status: Default::default(),
                });
                c.request_repaint();
            });
            worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            let json = serde_json::to_string(&request).map_err(|e| e.to_string())?;
            worker
                .post_message(&JsValue::from_str(&json))
                .map_err(|e| format!("{e:?}"))?;
            let (s, c, w) = (slot.clone(), ctx.clone(), worker.clone());
            let on_timeout = Closure::<dyn FnMut()>::new(move || {
                if s.borrow().is_none() {
                    w.terminate();
                    *s.borrow_mut() = Some(Outcome::failure(
                        StopReason::TimeBudget,
                        "wall-clock budget exhausted; browser worker terminated",
                        Limits::default().time_seconds,
                    ));
                    c.request_repaint();
                }
            });
            let timeout_id = web_sys::window()
                .ok_or("no browser window")?
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    on_timeout.as_ref().unchecked_ref(),
                    (Limits::default().time_seconds * 1000.0) as i32,
                )
                .map_err(|e| format!("could not schedule worker deadline: {e:?}"))?;
            Ok(Job {
                worker,
                slot,
                _on_message: on_message,
                _on_error: on_error,
                _on_timeout: on_timeout,
                timeout_id,
            })
        }

        pub fn poll(&mut self) -> Option<Outcome> {
            let result = self.slot.borrow_mut().take();
            if result.is_some() {
                if let Some(window) = web_sys::window() {
                    window.clear_timeout_with_handle(self.timeout_id);
                }
            }
            result
        }

        pub const STOP_ENDS_WORK: bool = true;

        pub fn stop(self) {
            self.worker.terminate();
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(self.timeout_id);
            }
            self.worker.terminate();
        }
    }
}

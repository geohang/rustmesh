// Meshes away from the page, so the app keeps responding: the same
// WebAssembly module as the app, called through mesh_job() (src/jobs.rs).
// The app stops a mesh by terminating this worker.
import init, { mesh_job } from "./rustmesh_app.js";

const ready = init();

self.onmessage = async (event) => {
  await ready;
  self.postMessage(mesh_job(event.data));
};

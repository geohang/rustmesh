//! Opening and saving files: the system's file dialogs on the desktop; the
//! browser's file picker and downloads on the web, where nothing is uploaded
//! anywhere. A picked file arrives in the [`Inbox`] (at once on the
//! desktop, a moment later in the browser).

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;

/// What a picked file is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Project,
    Domain,
    Points,
}

pub struct Picked {
    pub purpose: Purpose,
    pub name: String,
    pub text: Result<String, String>,
}

pub type Inbox = Rc<RefCell<Vec<Picked>>>;

/// File-type filter: a label and extensions without dots.
pub type Filter = (&'static str, &'static [&'static str]);

pub const PROJECT: Filter = ("rustmesh project", &["json"]);
pub const TABLE: Filter = ("Text or CSV table", &["csv", "txt", "dat", "xyz"]);

#[cfg(not(target_arch = "wasm32"))]
pub fn open(_ctx: &egui::Context, inbox: &Inbox, purpose: Purpose, filter: Filter) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter(filter.0, filter.1)
        .pick_file()
    {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("could not read {name}: {e}"));
        inbox.borrow_mut().push(Picked {
            purpose,
            name,
            text,
        });
    }
}

#[cfg(target_arch = "wasm32")]
pub fn open(ctx: &egui::Context, inbox: &Inbox, purpose: Purpose, filter: Filter) {
    let (ctx, inbox) = (ctx.clone(), inbox.clone());
    wasm_bindgen_futures::spawn_local(async move {
        if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter(filter.0, filter.1)
            .pick_file()
            .await
        {
            let name = file.file_name();
            let text = String::from_utf8(file.read().await)
                .map_err(|_| format!("{name} is not a text file"));
            inbox.borrow_mut().push(Picked {
                purpose,
                name,
                text,
            });
            ctx.request_repaint();
        }
    });
}

/// Saves what `write` produces under a name the user picks (desktop) or as
/// a download (web). `Ok(None)` when the user cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(
    name: &str,
    filter: Filter,
    write: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>,
) -> Result<Option<String>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_file_name(name)
        .add_filter(filter.0, filter.1)
        .save_file()
    else {
        return Ok(None);
    };
    let shown = path.display().to_string();
    let file =
        std::fs::File::create(&path).map_err(|e| format!("could not create {shown}: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    write(&mut out)
        .and_then(|_| std::io::Write::flush(&mut out))
        .map_err(|e| format!("could not write {shown}: {e}"))?;
    Ok(Some(shown))
}

#[cfg(target_arch = "wasm32")]
pub fn save(
    name: &str,
    _filter: Filter,
    write: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>,
) -> Result<Option<String>, String> {
    use wasm_bindgen::JsCast;

    let mut bytes = Vec::new();
    write(&mut bytes).map_err(|e| e.to_string())?;
    let fail = |e: wasm_bindgen::JsValue| format!("the browser could not save {name}: {e:?}");
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(fail)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no web page to save from")?;
    let link: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(fail)?
        .dyn_into()
        .map_err(|_| "no download link")?;
    link.set_href(&url);
    link.set_download(name);
    link.click();
    web_sys::Url::revoke_object_url(&url).map_err(fail)?;
    Ok(Some(format!("{name} (in your downloads)")))
}

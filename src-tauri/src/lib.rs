//! The native shell. Every command here is a thin pass-through to
//! `pallet_core::warehouse::Warehouse`, which owns all paths and checks.
//! The WebView sends opaque IDs and names, never paths (SAFETY.md S1, S7).
//! The folder picker runs here, in Rust, so choosing a warehouse doesn't
//! need a path-accepting command either.

use pallet_core::model::{Floor, HistoryEntry, TxOutcome, WarehouseStatus};
use serde::Serialize;
use std::sync::Mutex;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BootInfo {
    mode: &'static str,
    note: Option<String>,
    status: Option<WarehouseStatus>,
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use pallet_core::warehouse::Warehouse;
    use pallet_core::worker::Worker;
    use std::path::PathBuf;
    use tauri::{AppHandle, Manager, State};

    /// One warehouse at a time, owned by its confined worker thread (S1, S3);
    /// the `.pallet/lock` flock keeps other PALLET processes out.
    #[derive(Default)]
    pub struct Session(pub Mutex<Option<(Worker, PathBuf)>>);

    type Res<T> = Result<T, String>;

    fn with<T: Send + 'static>(s: &State<Session>, f: impl FnOnce(&mut Warehouse) -> T + Send + 'static) -> Res<T> {
        let guard = s.0.lock().map_err(|_| "The warehouse service stopped after an error. Restart PALLET.".to_string())?;
        let (w, _) = guard.as_ref().ok_or("No warehouse is open.")?;
        w.run(f)
    }

    fn sample_dir(app: &AppHandle) -> Res<PathBuf> {
        Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("sample-warehouse"))
    }

    fn open_into(s: &State<Session>, path: &std::path::Path, sample: bool) -> Res<WarehouseStatus> {
        let mut guard = s.0.lock().map_err(|_| "The warehouse service stopped after an error. Restart PALLET.".to_string())?;
        let canonical = std::fs::canonicalize(path).map_err(|e| format!("Can't open that folder: {e}."))?;
        if let Some((w, root)) = guard.as_ref() {
            if *root == canonical {
                return w.run(|w| w.status());
            }
        }
        let w = Worker::open(&canonical, sample).map_err(|e| e.message)?;
        let status = w.run(|w| w.status())?;
        // The previous warehouse (and its lock) is released only now that the new one opened.
        *guard = Some((w, canonical));
        Ok(status)
    }

    #[tauri::command]
    pub fn boot(app: AppHandle, s: State<Session>) -> BootInfo {
        let attempt = (|| -> Res<WarehouseStatus> {
            let dir = sample_dir(&app)?;
            pallet_core::sample::seed(&dir).map_err(|e| format!("Couldn't create the sample warehouse: {e}"))?;
            open_into(&s, &dir, true)
        })();
        match attempt {
            Ok(status) => BootInfo { mode: "real", note: None, status: Some(status) },
            Err(e) => BootInfo {
                mode: "virtual",
                note: Some(format!("Real-file mode is off: {e} You're driving a virtual sample instead.")),
                status: None,
            },
        }
    }

    #[tauri::command]
    pub fn status(s: State<Session>) -> Res<WarehouseStatus> {
        with(&s, |w| w.status())
    }

    #[tauri::command]
    pub fn list_floor(s: State<Session>, bay: Option<String>) -> Res<Floor> {
        with(&s, move |w| w.list_floor(bay.as_deref()))?
    }

    #[tauri::command]
    pub fn deliver(s: State<Session>, tx: String, cargo: String, bay: String, new_name: Option<String>) -> Res<TxOutcome> {
        with(&s, move |w| w.deliver(&tx, &cargo, &bay, new_name.as_deref(), None))
    }

    #[tauri::command]
    pub fn deliver_batch(s: State<Session>, batch: String, items: Vec<(String, String)>, bay: String) -> Res<Vec<TxOutcome>> {
        with(&s, move |w| w.deliver_batch(&batch, &items, &bay))
    }

    #[tauri::command]
    pub fn undo(s: State<Session>, tx: String, target: String, new_name: Option<String>) -> Res<TxOutcome> {
        with(&s, move |w| w.undo(&tx, &target, new_name.as_deref()))
    }

    #[tauri::command]
    pub fn duplicate(s: State<Session>, tx: String, cargo: String, new_name: Option<String>) -> Res<TxOutcome> {
        with(&s, move |w| w.duplicate(&tx, &cargo, new_name.as_deref()))
    }

    #[tauri::command]
    pub fn save_incident(s: State<Session>, tx: String, text: String) -> Res<TxOutcome> {
        with(&s, move |w| w.save_incident(&tx, &text))
    }

    #[tauri::command]
    pub fn tx_status(s: State<Session>, tx: String) -> Res<Option<TxOutcome>> {
        with(&s, move |w| w.tx_status(&tx))
    }

    #[tauri::command]
    pub fn history(s: State<Session>) -> Res<Vec<HistoryEntry>> {
        with(&s, |w| w.history())
    }

    #[tauri::command]
    pub fn acknowledge(s: State<Session>) -> Res<WarehouseStatus> {
        with(&s, |w| w.acknowledge())?
    }

    /// Shows the native folder picker from Rust. Async so the blocking dialog
    /// doesn't run on the main thread.
    #[tauri::command]
    pub async fn choose_warehouse(app: AppHandle, s: State<'_, Session>) -> Res<Option<WarehouseStatus>> {
        use tauri_plugin_dialog::DialogExt;
        let Some(picked) = app.dialog().file().set_title("Choose a warehouse folder").blocking_pick_folder() else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        open_into(&s, &path, false).map(Some)
    }

    /// Creates (or reopens) `~/PALLET-Warehouse`, the default S1 offers.
    #[tauri::command]
    pub async fn create_warehouse(app: AppHandle, s: State<'_, Session>) -> Res<WarehouseStatus> {
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        let dir = home.join("PALLET-Warehouse");
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("Couldn't create {}: {e}", dir.display())),
        }
        open_into(&s, &dir, false)
    }

    #[tauri::command]
    pub async fn open_sample(app: AppHandle, s: State<'_, Session>) -> Res<WarehouseStatus> {
        let dir = sample_dir(&app)?;
        pallet_core::sample::seed(&dir).map_err(|e| e.to_string())?;
        open_into(&s, &dir, true)
    }
}

/// Platforms without a tested adapter: the frontend runs its virtual sample,
/// and every file command reports that real-file mode is unavailable.
#[cfg(not(target_os = "linux"))]
mod native {
    use super::*;

    #[derive(Default)]
    pub struct Session(pub Mutex<()>);

    const OFF: &str = "Real-file mode isn't available on this platform.";

    #[tauri::command]
    pub fn boot() -> BootInfo {
        BootInfo { mode: "virtual", note: pallet_core::unsupported_reason().map(str::to_string), status: None }
    }
    #[tauri::command]
    pub fn status() -> Result<WarehouseStatus, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn list_floor() -> Result<Floor, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn deliver() -> Result<TxOutcome, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn deliver_batch() -> Result<Vec<TxOutcome>, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn undo() -> Result<TxOutcome, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn duplicate() -> Result<TxOutcome, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn save_incident() -> Result<TxOutcome, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn tx_status() -> Result<Option<TxOutcome>, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn history() -> Result<Vec<HistoryEntry>, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn acknowledge() -> Result<WarehouseStatus, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn choose_warehouse() -> Result<Option<WarehouseStatus>, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn create_warehouse() -> Result<WarehouseStatus, String> {
        Err(OFF.into())
    }
    #[tauri::command]
    pub fn open_sample() -> Result<WarehouseStatus, String> {
        Err(OFF.into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use native::*;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .invoke_handler(tauri::generate_handler![
            boot,
            status,
            list_floor,
            deliver,
            deliver_batch,
            undo,
            duplicate,
            save_incident,
            tx_status,
            history,
            acknowledge,
            choose_warehouse,
            create_warehouse,
            open_sample
        ])
        .run(tauri::generate_context!())
        .expect("error while running PALLET");
}

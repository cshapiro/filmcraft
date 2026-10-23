//! User-confirmed destinations for Premiere imports. Ordinary media never opens a folder picker.
use crate::{FilmcraftApp, native_dialogs::FileDialog};
use serde_json::{Value, json};
use std::path::Path;

fn is_premiere(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("prproj"))
}

impl FilmcraftApp {
    /// Collect the destination before importing anything, including mixed file batches.
    /// A caller supplying an explicit destination bypasses the native picker.
    pub(crate) fn import_paths(&mut self, params: Value, pointer: Option<egui::Pos2>) -> Result<Value, String> {
        let paths: Vec<String> = match params.get("paths") {
            Some(value) => serde_json::from_value(value.clone()).map_err(|error| format!("Invalid import paths: {error}"))?,
            None => params.get("path").and_then(Value::as_str).map(str::to_owned).into_iter().collect(),
        };
        if !paths.iter().any(|path| is_premiere(path)) || params.get("destination").is_some() || params.get("destinationRoot").is_some() {
            return self.finish_path_import(params, pointer);
        }
        let remembered = &self.session.prefs.general.premiere_import_directory;
        let mut at = if remembered.is_empty() {
            self.hooks.projects_directory.clone()
        } else {
            Path::new(remembered).ancestors().find(|path| path.is_dir()).map(|path| path.to_string_lossy().into_owned())
        };
        if !(FileDialog::Folder { at: at.clone() }).available(&self.hooks) {
            return Err("Premiere import requires an explicit destination on this host".into());
        }
        // Only an empty managed first-use folder is prepared before confirmation. If it is
        // unavailable, the picker still lets the user choose another location.
        if remembered.is_empty()
            && let Some(suggested) = at.as_deref()
            && std::fs::create_dir_all(suggested).is_err()
        {
            at = Path::new(suggested).ancestors().find(|path| path.is_dir()).map(|path| path.to_string_lossy().into_owned());
        }
        self.pick(FileDialog::Folder { at }, move |app, chosen| {
            let Some(parent) = chosen.into_iter().next() else { return Ok(Value::Null) };
            let mut prefs = app.session.prefs.clone();
            prefs.general.premiere_import_directory = parent.clone();
            app.session.set_prefs(prefs).map_err(|error| format!("Cannot remember the import folder: {error}"))?;
            let mut params = params;
            params["destinationRoot"] = json!(parent);
            app.finish_path_import(params, pointer)
        })
    }

    fn finish_path_import(&mut self, params: Value, pointer: Option<egui::Pos2>) -> Result<Value, String> {
        let result = self.session.execute("file.import", params).map_err(|error| error.to_string())?;
        self.after_path_import(&result, pointer);
        Ok(result)
    }

    fn after_path_import(&mut self, result: &Value, pointer: Option<egui::Pos2>) {
        if let Some(errors) = result.get("errors").and_then(Value::as_array)
            && !errors.is_empty()
        {
            self.ui.status = errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ");
        }
        if let Some(pointer) = pointer
            && self.session.prefs.timeline.drop_import_to_timeline
        {
            let items: Vec<_> =
                result.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).map(filmcraft_engine::project::ItemId).collect();
            if !items.is_empty() {
                self.pending_timeline_drop = Some((items, pointer));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_engine::Session;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn fixture(name: &str) -> (PathBuf, String) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/premiere-picker").join(format!("{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("example.prproj");
        std::fs::write(&source, br#"<PremiereData><Sequence ObjectUID="s"><Name>Sequence</Name></Sequence></PremiereData>"#).unwrap();
        (dir, source.to_string_lossy().into_owned())
    }

    #[test]
    fn cancelled_batch_import_never_changes_project_or_source_folder() {
        let (dir, source) = fixture("cancel");
        let mut app = FilmcraftApp::new(Session::default());
        let before = app.session.project.clone();
        let initial = dir.join("managed");
        app.hooks.projects_directory = Some(initial.to_string_lossy().into_owned());
        let seen = Arc::new(Mutex::new(String::new()));
        let captured = seen.clone();
        app.hooks.pick_folder_at = Some(Box::new(move |at| {
            *captured.lock().unwrap() = at.to_owned();
            None
        }));
        let result = app.import_paths(json!({"paths":[source,"missing.wav"]}), None).unwrap();
        assert_eq!(result, Value::Null);
        assert_eq!(app.session.project.as_ref(), before.as_ref());
        assert_eq!(*seen.lock().unwrap(), initial.to_string_lossy());
        assert!(app.session.prefs.general.premiere_import_directory.is_empty());
        assert!(!dir.join("example.filmcraft-import-1").exists());
        assert!(std::fs::read_dir(initial).unwrap().next().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chosen_folder_is_remembered_and_occupied_imports_are_preserved() {
        let (dir, source) = fixture("remember");
        let chosen = dir.join("chosen");
        std::fs::create_dir_all(&chosen).unwrap();
        let occupied = chosen.join("example.filmcraft-import-1");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), b"keep").unwrap();
        let mut app = FilmcraftApp::new(Session::default());
        let next = chosen.to_string_lossy().into_owned();
        app.hooks.pick_folder = Some(Box::new(move || Some(next.clone())));
        let before = std::fs::read(&source).unwrap();
        let result = app.import_paths(json!({"paths":[source]}), None).unwrap();
        assert_eq!(result["errors"], json!([]));
        assert_eq!(std::fs::read(chosen.join("example.filmcraft-import-2/source.prproj")).unwrap(), before);
        assert_eq!(std::fs::read(occupied.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(std::fs::read(&source).unwrap(), before);
        let save_seen = Arc::new(Mutex::new(String::new()));
        let save_captured = save_seen.clone();
        app.hooks.pick_save = Some(Box::new(move |name| {
            *save_captured.lock().unwrap() = name.to_owned();
            None
        }));
        app.file_dialog("file.save", &json!({})).unwrap();
        assert_eq!(
            Path::new(&*save_seen.lock().unwrap()).parent().unwrap().canonicalize().unwrap(),
            chosen.join("example.filmcraft-import-2").canonicalize().unwrap()
        );
        assert!(app.session.path.is_none());
        let prefs_path = dir.join("preferences.json");
        app.session.prefs.save(&prefs_path).unwrap();
        let restored = filmcraft_engine::autosave::Preferences::load(&prefs_path);
        assert_eq!(restored.general.premiere_import_directory, chosen.to_string_lossy());
        let mut next_session = Session::default();
        next_session.prefs = restored;
        app = FilmcraftApp::new(next_session);
        let seen = Arc::new(Mutex::new(String::new()));
        let captured = seen.clone();
        app.hooks.pick_folder_at = Some(Box::new(move |at| {
            *captured.lock().unwrap() = at.to_owned();
            None
        }));
        app.import_paths(json!({"paths":[source]}), None).unwrap();
        assert_eq!(*seen.lock().unwrap(), chosen.to_string_lossy());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn asynchronous_folder_cancellation_has_no_import_side_effects() {
        let (dir, source) = fixture("async");
        let mut app = FilmcraftApp::new(Session::default());
        let before = app.session.project.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut rx = Some(rx);
        app.hooks.run_dialog = Some(Box::new(move |dialog| {
            assert!(matches!(dialog, FileDialog::Folder { .. }));
            rx.take().unwrap()
        }));
        assert_eq!(app.import_paths(json!({"paths":[source]}), None).unwrap(), json!({"dialog":"open"}));
        assert!(app.native_dialog_open());
        tx.send(Vec::new()).unwrap();
        app.poll_native_dialog(&egui::Context::default());
        assert!(!app.native_dialog_open());
        assert_eq!(app.session.project.as_ref(), before.as_ref());
        assert!(!dir.join("example.filmcraft-import-1").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ordinary_media_does_not_open_the_destination_picker() {
        let mut app = FilmcraftApp::new(Session::default());
        app.hooks.pick_folder = Some(Box::new(|| panic!("ordinary media must not ask for an import folder")));
        assert!(app.import_paths(json!({"paths":["missing.wav"]}), None).is_err());
    }

    #[test]
    fn multiple_projects_get_distinct_owned_folders() {
        let (dir, source) = fixture("multiple");
        let mut app = FilmcraftApp::new(Session::default());
        let next = dir.to_string_lossy().into_owned();
        app.hooks.pick_folder = Some(Box::new(move || Some(next.clone())));
        let result = app.import_paths(json!({"paths":[source,source]}), None).unwrap();
        assert_eq!(result["errors"], json!([]));
        assert_eq!(result["documents"].as_array().unwrap().len(), 2);
        assert!(dir.join("example.filmcraft-import-1/source.prproj").exists());
        assert!(dir.join("example.filmcraft-import-2/source.prproj").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

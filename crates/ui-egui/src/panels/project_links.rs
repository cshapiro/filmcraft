//! Explicitly enable the locally installed renderer and choose a linked project object.
use crate::FilmcraftApp;
use egui::{Context, Id};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Dialog {
    executable: String,
    path: String,
    objects: Vec<Value>,
    object: String,
    message: String,
    bake_path: String,
}

fn id() -> Id {
    Id::new("linkedProjects.dialog")
}

pub fn open(ctx: &Context) {
    ctx.data_mut(|d| {
        d.insert_temp(id(), Dialog::default());
    });
}

pub fn show(app: &mut FilmcraftApp, ctx: &Context) {
    let Some(mut state) = ctx.data_mut(|d| d.get_temp::<Dialog>(id())) else { return };
    let mut open = true;
    egui::Window::new("Linked Projects (experimental)").id(id()).open(&mut open).default_width(580.0).show(ctx, |ui| {
        ui.label("EffectCraft is the first adapter. Saved edits refresh in the background.");
        ui.label("Enable a trusted installed effectcraft-cli for this session:");
        let field = ui.add(egui::TextEdit::singleline(&mut state.executable).desired_width(f32::INFINITY));
        app.auto.add_response("linkedProjects.renderer", &field, "Renderer executable");
        let enable = ui.button("Enable renderer");
        app.auto.add_response("linkedProjects.enable", &enable, "Enable renderer");
        if enable.clicked() {
            state.message = match app.session.execute("linkedProject.configure", json!({"executable":state.executable})) {
                Ok(_) => "Renderer enabled for this session".into(),
                Err(e) => e.to_string(),
            };
        }
        ui.separator();
        ui.label("EffectCraft project path (.ecproj):");
        let field = ui.add(egui::TextEdit::singleline(&mut state.path).desired_width(f32::INFINITY));
        app.auto.add_response("linkedProjects.path", &field, "Project path");
        let inspect = ui.button("List compositions");
        app.auto.add_response("linkedProjects.inspect", &inspect, "List compositions");
        if inspect.clicked() {
            match app.session.execute("linkedProject.inspect", json!({"path":state.path})) {
                Ok(v) => {
                    state.objects = v["objects"].as_array().cloned().unwrap_or_default();
                    state.object = state.objects.first().and_then(|o| o["id"].as_str()).unwrap_or("").into();
                    state.message.clear();
                }
                Err(e) => state.message = e.to_string(),
            }
        }
        egui::ScrollArea::vertical().id_salt("linkedProjects.objects").max_height(160.0).show(ui, |ui| {
            for object in &state.objects {
                let key = object["id"].as_str().unwrap_or("");
                let choice = ui.radio_value(&mut state.object, key.into(), object["name"].as_str().unwrap_or("Composition"));
                app.auto.add_response(&format!("linkedProjects.object.{key}"), &choice, "Composition");
            }
        });
        let import = ui.add_enabled(!state.object.is_empty(), egui::Button::new("Import linked composition"));
        app.auto.add_response("linkedProjects.import", &import, "Import linked composition");
        if import.clicked() {
            let bin = app.import_bin().0;
            state.message = match app.session.execute("linkedProject.import", json!({"path":state.path,"object":state.object,"bin":bin})) {
                Ok(_) => "Linked item added to Project; rendering in background".into(),
                Err(e) => e.to_string(),
            };
        }
        ui.label(&state.message);
        ui.separator();
        let status = app.session.execute("linkedProject.status", json!({})).unwrap_or(Value::Null);
        if let Some(message) = status["progress"].as_str() {
            ui.spinner();
            ui.label(message);
        }
        ui.label("New .mov destination for Bake (existing files are never overwritten):");
        let field = ui.add(egui::TextEdit::singleline(&mut state.bake_path).desired_width(f32::INFINITY));
        app.auto.add_response("linkedProjects.bakePath", &field, "Bake destination");
        egui::ScrollArea::vertical().id_salt("linkedProjects.items").max_height(200.0).show(ui, |ui| {
            for row in status["items"].as_array().into_iter().flatten() {
                let item = row["item"].as_u64().unwrap_or(0);
                ui.group(|ui| {
                    ui.label(row["link"]["object_name"].as_str().unwrap_or("Linked item"));
                    ui.label(row["status"]["message"].as_str().unwrap_or("Waiting for renderer"));
                    ui.horizontal(|ui| {
                        for (suffix, label, command, params) in [
                            ("bake", "Bake", "linkedProject.bake", json!({"item":item,"path":state.bake_path})),
                            ("restore", "Restore link", "linkedProject.bake", json!({"item":item,"restore":true})),
                            ("relink", "Relink to path above", "linkedProject.relink", json!({"item":item,"path":state.path,"object":state.object})),
                        ] {
                            let button = ui.button(label);
                            app.auto.add_response(&format!("linkedProjects.{item}.{suffix}"), &button, label);
                            if button.clicked()
                                && let Err(e) = app.session.execute(command, params)
                            {
                                state.message = e.to_string();
                            }
                        }
                    });
                });
            }
        });
        ui.horizontal(|ui| {
            for (suffix, label, command) in [("refresh", "Refresh", "linkedProject.refresh"), ("cancel", "Cancel render", "linkedProject.cancel")] {
                let button = ui.button(label);
                app.auto.add_response(&format!("linkedProjects.{suffix}"), &button, label);
                if button.clicked()
                    && let Err(e) = app.session.execute(command, json!({}))
                {
                    state.message = e.to_string();
                }
            }
        });
        if status["running"].as_bool() == Some(true) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    });
    if open {
        ctx.data_mut(|d| d.insert_temp(id(), state));
    } else {
        ctx.data_mut(|d| d.remove::<Dialog>(id()));
    }
}

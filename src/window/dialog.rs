use eframe::wgpu;
use egui_notify::Toast;
use egui_tiles::TileId;
use silicate_compositor::buffer::BufferDimensions;
use std::sync::mpsc::Sender;

use crate::app::AppEvent;
use crate::app::export::LayerSource;
use crate::platform;
use silica_gpu::ProcreateFile;
use silicate_compositor::pipeline::Pipeline;

pub struct Dialog {
    event_sender: Sender<AppEvent>,
}

impl Dialog {
    pub fn new(event_sender: Sender<AppEvent>) -> Self {
        Self { event_sender }
    }

    fn send_toast(&self, toast: Toast) {
        self.event_sender.send(AppEvent::Toast(toast)).ok();
    }

    pub async fn load_dialog(self, tabs: Option<TileId>) {
        let source = match platform::pick_file().await {
            Ok(Some(source)) => source,
            Ok(None) => {
                self.send_toast(Toast::info("Load cancelled."));
                return;
            }
            Err(err) => {
                log::error!("Failed to read file: {err}");
                self.send_toast(Toast::error("Failed to read file."));
                return;
            }
        };

        self.event_sender
            .send(AppEvent::LoadFile { source, tabs })
            .unwrap();
    }

    pub async fn save_dialog(
        self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        copied_texture: wgpu::Texture,
    ) {
        let Some(target) = platform::pick_save_target().await else {
            self.send_toast(Toast::info("Export cancelled."));
            return;
        };
        let name = target.name();

        let dim = BufferDimensions::from_extent(copied_texture.size());
        let result = match crate::app::App::export(&copied_texture, &device, &queue, dim).await {
            Ok(image) => target.save(image).await,
            Err(err) => Err(err.to_string()),
        };

        match result {
            Ok(()) => self.send_toast(Toast::success(format!(
                "File {name} successfully exported."
            ))),
            Err(err) => self.send_toast(Toast::error(format!(
                "File {name} failed to export. Reason: {err}."
            ))),
        }
    }

    pub async fn export_layers_dialog(
        self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        pipeline: Pipeline,
        file: ProcreateFile,
        source: LayerSource,
    ) {
        let file_name = format!("{}.zip", file.name.as_deref().unwrap_or("Untitled Artwork"));
        let Some(target) = platform::pick_archive_target(&file_name).await else {
            self.send_toast(Toast::info("Export cancelled."));
            return;
        };
        let name = target.name();

        let result = match crate::app::export::export_layers(
            &device, &queue, &pipeline, source, &file,
        )
        .await
        {
            Ok(bytes) => target.save_bytes(bytes).await,
            Err(err) => Err(err),
        };

        match result {
            Ok(()) => self.send_toast(Toast::success(format!(
                "Layers successfully exported to {name}."
            ))),
            Err(err) => self.send_toast(Toast::error(format!(
                "Layers failed to export to {name}. Reason: {err}."
            ))),
        }
    }
}

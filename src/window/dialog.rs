use eframe::wgpu;
use egui_notify::Toast;
use egui_tiles::TileId;
use silicate_compositor::buffer::BufferDimensions;
use std::sync::mpsc::Sender;

use crate::app::AppEvent;
use crate::platform;

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
}

mod app;
mod gui;
mod window;

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

mod platform;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use platform::init_thread_pool;

use app::AppEvent;
use std::sync::mpsc::{Receiver, Sender};
use window::AppInstance;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

pub struct AppMultiplexer {
    rt: platform::Runtime,
    #[cfg(not(target_arch = "wasm32"))]
    initial_files: Vec<PathBuf>,
    running: Option<AppInstance>,
    event_sender: Sender<AppEvent>,
    event_receiver: Receiver<AppEvent>,
}

impl AppMultiplexer {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = std::sync::mpsc::channel();
        Self {
            rt: platform::Runtime::new(),
            #[cfg(not(target_arch = "wasm32"))]
            initial_files: Vec::new(),
            running: None,
            event_sender,
            event_receiver,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_initial_files(&mut self, initial_files: Vec<PathBuf>) {
        self.initial_files = initial_files;
    }
}

impl eframe::App for AppMultiplexer {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Initialize the app instance if not already done
        if self.running.is_none() {
            if let Some(wgpu_render_state) = frame.wgpu_render_state() {
                let device = wgpu_render_state.device.clone();
                let queue = wgpu_render_state.queue.clone();

                #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
                let mut instance =
                    AppInstance::new_for_eframe(device, queue, ctx, &self.event_sender);

                #[cfg(not(target_arch = "wasm32"))]
                instance.load_files(std::mem::take(&mut self.initial_files));

                self.running = Some(instance);
            }
        }

        // Process user events from the channel
        while let Ok(app_event) = self.event_receiver.try_recv() {
            if let Some(app) = self.running.as_mut() {
                app.handle_user_event(app_event, &self.rt, ctx, frame);
            }
        }

    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        // Render the GUI
        if let Some(app) = self.running.as_mut() {
            for (_, (compositor, texture)) in app.compositors_mut() {
                compositor.rendering_tick_blocking(texture);
            }
            app.render_gui(ui);
        }
    }
}

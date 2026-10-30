use std::{
    borrow::Cow,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{ZipArchiveMmap, tiling::CanvasTiling, types::file::TileUpload};

pub(crate) struct LoadParams<'a> {
    pub(crate) archive: &'a ZipArchiveMmap<'a>,
    pub(crate) file_names: Vec<Cow<'a, str>>,
    pub(crate) tiling: CanvasTiling,
    /// Receives every decoded tile. Called from rayon worker threads.
    pub(crate) upload: &'a (dyn Fn(TileUpload) + Sync),
    pub(crate) chunk_id_counter: AtomicU32,
    pub(crate) layer_id_counter: AtomicU32,
}

impl LoadParams<'_> {
    pub fn allocate_layer_id(&self) -> u32 {
        self.layer_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    pub fn allocate_chunk_id(&self) -> u32 {
        self.chunk_id_counter.fetch_add(1, Ordering::Relaxed)
    }
}

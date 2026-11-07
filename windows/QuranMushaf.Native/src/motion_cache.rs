//! UI-thread-owned preparation of authentic book pixels between interactions.
//! CPU rasters are independent of a presentation device; uploaded GPU resources
//! belong to the caller and its device generation. No source assets are edited.
use crate::render::BookRaster;
use std::collections::VecDeque;

use crate::motion_policy::{MAX_CPU_PIXEL_BYTES, MAX_RASTERS};

#[derive(Clone, PartialEq, Eq)]
pub struct CpuRasterKey {
    pub page: u32,
    pub theme: u32,
    pub arabic: bool,
    pub contextual: bool,
    pub selected: Option<(u32, u32)>,
    pub recitation: Option<(u32, u32)>,
    pub opening: Option<u32>,
    pub dpi: u32,
    /// Exact original book x/y/width/height represented by f32::to_bits().
    /// Origin matters because it determines the glyphs' physical-pixel phase.
    pub geometry: [u32; 4],
    pub clock: u16,
    pub show_clock: bool,
    pub clock_24: bool,
    pub grayscale: bool,
}
impl CpuRasterKey {
    fn same_environment(&self, other: &Self) -> bool {
        // A fold can retain current and destination pages in both AA modes.
        // Geometry/environment changes invalidate the plan. Selection remains
        // an exact local lookup key, without discarding unselected neighbors.
        self.theme == other.theme
            && self.arabic == other.arabic
            && self.contextual == other.contextual
            && self.dpi == other.dpi
            && self.geometry == other.geometry
            && self.clock == other.clock
            && self.show_clock == other.show_clock
            && self.clock_24 == other.clock_24
    }
}

pub struct CpuRasterCache {
    entries: VecDeque<(CpuRasterKey, BookRaster)>,
    pixel_bytes: u64,
    byte_limit: u64,
}
impl Default for CpuRasterCache {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            pixel_bytes: 0,
            byte_limit: MAX_CPU_PIXEL_BYTES,
        }
    }
}
impl CpuRasterCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pixel_bytes = 0;
    }
    #[cfg(feature = "qa")]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Logical WIC pixel payload; COM objects and driver allocations are not
    /// included. A caller must count its live GPU textures separately.
    #[cfg(feature = "qa")]
    pub fn pixel_bytes(&self) -> u64 {
        self.pixel_bytes
    }
    /// Reserve active WIC and uploaded bitmap pixels before a new allocation.
    pub fn set_budget(&mut self, bytes: u64) {
        self.byte_limit = bytes.min(MAX_CPU_PIXEL_BYTES);
        while self.pixel_bytes > self.byte_limit {
            let Some((_, raster)) = self.entries.pop_front() else {
                break;
            };
            self.pixel_bytes -= pixel_bytes(&raster);
        }
    }
    pub fn retain_keys(&mut self, keys: &[CpuRasterKey]) {
        self.entries.retain(|(key, _)| keys.contains(key));
        self.pixel_bytes = self
            .entries
            .iter()
            .map(|(_, raster)| pixel_bytes(raster))
            .sum();
    }
    pub fn contains(&self, key: &CpuRasterKey) -> bool {
        self.entries.iter().any(|(stored, _)| stored == key)
    }
    /// Drop stale preparation while retaining the current/next pages and the
    /// separate grayscale reverse face for this exact rendering environment.
    pub fn retain_environment(&mut self, key: &CpuRasterKey) {
        self.entries
            .retain(|(stored, _)| stored.same_environment(key));
        self.pixel_bytes = self
            .entries
            .iter()
            .map(|(_, raster)| pixel_bytes(raster))
            .sum();
    }
    /// Take ownership, avoiding a duplicate WIC allocation when an animation
    /// moves prepared content into its device-specific presentation cache.
    pub fn take(&mut self, key: &CpuRasterKey) -> Option<BookRaster> {
        let position = self.entries.iter().position(|(stored, _)| stored == key)?;
        let (_, raster) = self.entries.remove(position)?;
        self.pixel_bytes -= pixel_bytes(&raster);
        Some(raster)
    }
    /// Keep at most five admitted rasters within the reserved CPU allowance.
    /// A rejected raster is dropped; callers preserve the normal vector path.
    pub fn insert(&mut self, key: CpuRasterKey, raster: BookRaster) -> bool {
        let bytes = pixel_bytes(&raster);
        let actual_geometry = [
            raster.bounds.x.to_bits(),
            raster.bounds.y.to_bits(),
            raster.bounds.w.to_bits(),
            raster.bounds.h.to_bits(),
        ];
        if bytes == 0
            || bytes > self.byte_limit
            || key.contextual != raster.contextual
            || key.grayscale != raster.grayscale
            || key.dpi != raster.dpi.to_bits()
            || key.geometry != actual_geometry
        {
            return false;
        }
        if let Some(position) = self.entries.iter().position(|(stored, _)| stored == &key) {
            if let Some((_, previous)) = self.entries.remove(position) {
                self.pixel_bytes -= pixel_bytes(&previous);
            }
        }
        while self.entries.len() >= MAX_RASTERS || self.pixel_bytes + bytes > self.byte_limit {
            let Some((_, previous)) = self.entries.pop_front() else {
                return false;
            };
            self.pixel_bytes -= pixel_bytes(&previous);
        }
        self.pixel_bytes += bytes;
        self.entries.push_back((key, raster));
        true
    }
}

fn pixel_bytes(raster: &BookRaster) -> u64 {
    u64::from(raster.pixel_size.width)
        .saturating_mul(u64::from(raster.pixel_size.height))
        .saturating_mul(4)
}

//! The horizontal mapping between time and screen position, shared by one editor's views (grid,
//! value lane, overview strip): Responsibility 4. Ported from the old app's `TimeAxis.cs` at
//! `c81774e` (dawsu!'s own code).

use dawsu_foundation::SongTime;

#[derive(Debug, Clone, PartialEq)]
pub struct TimeAxis {
    left_ms: f64,
    zoom: f64,
    /// Song length in ms: the scroll limit.
    pub end_ms: f64,
    /// Width of the grid in pixels, set by the view on layout.
    pub width_px: f64,
    version: u64,
}

impl Default for TimeAxis {
    fn default() -> Self {
        TimeAxis {
            left_ms: 0.0,
            zoom: 0.15,
            end_ms: 0.0,
            width_px: 1000.0,
            version: 0,
        }
    }
}

impl TimeAxis {
    pub const MIN_ZOOM: f64 = 0.01;
    pub const MAX_ZOOM: f64 = 2.0;
    pub const MIN_LEFT_MS: f64 = -2000.0;

    /// The zoom slider is logarithmic: equal steps zoom by equal factors. `v` runs 0 to 1.
    pub fn zoom_from_slider(v: f64) -> f64 {
        Self::MIN_ZOOM * (Self::MAX_ZOOM / Self::MIN_ZOOM).powf(v)
    }

    pub fn slider_from_zoom(zoom: f64) -> f64 {
        (zoom / Self::MIN_ZOOM).ln() / (Self::MAX_ZOOM / Self::MIN_ZOOM).ln()
    }

    /// Time at the grid's left edge.
    pub fn left(&self) -> SongTime {
        SongTime::from_ms(self.left_ms)
    }

    /// Clamped to -2000 ms .. the song end.
    pub fn set_left(&mut self, left: SongTime) {
        let value = left.ms().clamp(Self::MIN_LEFT_MS, self.end_ms.max(0.0));
        if value != self.left_ms {
            self.left_ms = value;
            self.version += 1;
        }
    }

    /// Pixels per ms.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        let value = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if value != self.zoom {
            self.zoom = value;
            self.version += 1;
        }
    }

    /// Bumps when left or zoom change; views compare it.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn visible_duration_ms(&self) -> f64 {
        self.width_px / self.zoom
    }

    pub fn x(&self, time: SongTime) -> f64 {
        (time.ms() - self.left_ms) * self.zoom
    }

    pub fn time(&self, x: f64) -> SongTime {
        SongTime::from_ms(self.left_ms + x / self.zoom)
    }

    /// Zooms by `factor`, keeping the time under `x` where it is.
    pub fn zoom_at(&mut self, factor: f64, x: f64) {
        let anchor = self.time(x).ms();
        self.set_zoom(self.zoom * factor);
        self.set_left(SongTime::from_ms(anchor - x / self.zoom));
    }
}

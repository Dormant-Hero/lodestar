//! Box layout with WPF's measure / arrange rules (`engine`). The engine knows boxes, text and
//! pictures, not XAML, so it lives here rather than in the Playnite importer that lays its views
//! out with it.

mod engine;

pub(crate) use engine::*;

//! Repeatable software-target costs; these are not display frame-pacing claims.
use crate::{
    data::{Resources, Result},
    render::{BookView, Palette, Rect, Renderer},
};
use std::{path::Path, time::Instant};
use windows::Win32::{
    Graphics::{
        Direct2D::{Common::*, *},
        Dxgi::Common::*,
        Imaging::*,
    },
    System::Com::*,
};

pub fn run(resources: &mut Resources, output: &Path) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let mut cases = Vec::new();
    for contextual in [false, true] {
        let mut renderer = Renderer::new(resources)?;
        renderer.contextual = contextual;
        renderer.show_clock = false;
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bitmap = wic.CreateBitmap(
                1280,
                900,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let target = renderer.factory.CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                    ..Default::default()
                },
            )?;
            let palette = Palette::new(0);
            let area = Rect {
                x: 48.,
                y: 38.,
                w: 1184.,
                h: 857.,
            };
            let cold = Instant::now();
            target.BeginDraw();
            target.Clear(Some(&palette.background));
            renderer.book(
                &target,
                resources,
                BookView {
                    page: 45,
                    area,
                    palette,
                    selection: None,
                    recitation: None,
                    opening: None,
                    arabic: true,
                },
            )?;
            target.EndDraw(None, None)?;
            let cold_ms = cold.elapsed().as_secs_f64() * 1000.;
            let mut samples = Vec::new();
            for frame in 0..48 {
                let progress = (frame % 24) as f32 / 23.;
                let start = Instant::now();
                target.BeginDraw();
                target.Clear(Some(&palette.background));
                renderer.fold(
                    &target,
                    resources,
                    BookView {
                        page: 47,
                        area,
                        palette,
                        selection: None,
                        recitation: None,
                        opening: None,
                        arabic: true,
                    },
                    45,
                    1,
                    progress,
                    false,
                )?;
                target.EndDraw(None, None)?;
                samples.push(start.elapsed().as_secs_f64() * 1000.);
            }
            let mut sorted = samples.clone();
            sorted.sort_by(f64::total_cmp);
            cases.push(
                serde_json::json!({"contextual":contextual,"coldBookMs":cold_ms,
                "foldSamplesMs":samples,"medianFoldMs":sorted[24],"p95FoldMs":sorted[45],
                "maximumFoldMs":sorted[47]}),
            );
        }
    }
    std::fs::write(
        output.join("render-costs.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"1280x900 WIC software raster target including EndDraw; no HWND/compositor/input pacing measured",
            "cases":cases,"passed":true
        }))?,
    )?;
    Ok(())
}

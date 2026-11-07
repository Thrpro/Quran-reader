#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_op_in_unsafe_fn)]
mod app_core;
mod audio;
mod audio_cache;
mod chrome;
mod combo_control;
mod control_vectors;
mod data;
mod field_style;
mod file_integrity;
mod fluent_icons;
mod frame_clock;
mod guided_tour;
mod http_security;
#[cfg(feature = "qa")]
mod large_handoff_qa;
mod list_scroll;
mod listening_layout;
mod motion_cache;
mod motion_controls;
mod motion_policy;
mod mp3;
mod outlines;
mod owned_file;
mod page_turn;
mod profile_file;
mod quran_shape;
mod range_stream;
mod reader_state;
mod reading_control;
mod render;
#[cfg(feature = "qa")]
mod render_benchmark;
#[cfg(feature = "qa")]
mod scoped_cache_qa;
mod scoped_store;
mod search_layout;
mod smooth_scroll;
#[cfg(feature = "qa")]
mod streamed_audio_qa;
mod tafsir_library;
mod timed_audio;
mod timed_store;
mod tooltips;
mod ui;
use data::{Resources, Result};
use std::{path::PathBuf, time::Instant};
use windows::{
    Win32::{
        System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    },
    core::HSTRING,
};

fn root() -> Result<PathBuf> {
    #[cfg(feature = "qa")]
    {
        let args: Vec<_> = std::env::args().collect();
        if let Some(i) = args.iter().position(|s| s == "--qa-resources") {
            return Ok(
                PathBuf::from(args.get(i + 1).ok_or("Missing QA resources directory")?)
                    .canonicalize()?,
            );
        }
    }
    let executable = std::env::current_exe()?;
    let packaged = executable
        .parent()
        .ok_or("Missing executable directory")?
        .join("Resources");
    if packaged
        .join("experimental/digital-khatt/pages/p1.json")
        .is_file()
    {
        return Ok(packaged);
    }
    #[cfg(any(debug_assertions, feature = "qa"))]
    {
        let developer = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public-qul");
        if developer
            .join("experimental/digital-khatt/pages/p1.json")
            .is_file()
        {
            return Ok(developer.canonicalize()?);
        }
    }
    Err("The authentic Quran resources are missing. Extract the entire native preview folder before opening the app.".into())
}
fn run() -> Result<()> {
    let began = Instant::now();
    unsafe {
        RoInitialize(RO_INIT_SINGLETHREADED)?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            control_vectors::shutdown();
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let resources = Resources::open(root()?)?;
    #[cfg(feature = "qa")]
    let mut resources = resources;
    #[cfg(feature = "qa")]
    {
        let args: Vec<_> = std::env::args().collect();
        if let Some(index) = args.iter().position(|s| s == "--verify-range-stream") {
            return audio::Audio::range_stream_smoke(&PathBuf::from(
                args.get(index + 1).ok_or("Missing range stream output")?,
            ));
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-opening-render") {
            let out = PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing opening rendering output")?,
            );
            return render::Renderer::new(&mut resources)?.opening_snapshots(&mut resources, &out);
        }
        if let Some(index) = args
            .iter()
            .position(|argument| argument == "--qa-prepare-scoped-cache")
        {
            return scoped_cache_qa::prepare(
                &resources,
                &PathBuf::from(args.get(index + 1).ok_or("Missing scoped cache output")?),
            );
        }
        if let Some(index) = args.iter().position(|s| s == "--benchmark-render") {
            return render_benchmark::run(
                &mut resources,
                &PathBuf::from(args.get(index + 1).ok_or("Missing benchmark output")?),
            );
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-streamed-audio") {
            return streamed_audio_qa::verify(
                &PathBuf::from(args.get(index + 1).ok_or("Missing recording path")?),
                &PathBuf::from(args.get(index + 2).ok_or("Missing streamed audio output")?),
            );
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-timed-audio") {
            let out = PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing timed audio prototype folder")?,
            );
            return audio::Audio::new()?.timed_smoke(&out);
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-live-rate") {
            return audio::Audio::live_rate_smoke(&PathBuf::from(
                args.get(index + 1).ok_or("Missing live rate output")?,
            ));
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-timed-store") {
            return audio::Audio::timed_store_smoke(&PathBuf::from(
                args.get(index + 1).ok_or("Missing timed store output")?,
            ));
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-scoped-store") {
            return timed_store::scoped_smoke(&PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing rolling recording output")?,
            ));
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-long-timed-audio") {
            return audio::Audio::long_timed_smoke(&PathBuf::from(
                args.get(index + 1).ok_or("Missing long audio output")?,
            ));
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-audio") {
            let out = PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing audio QA output directory")?,
            );
            std::fs::create_dir_all(&out)?;
            let report = audio::Audio::new()?.smoke(&out)?;
            std::fs::write(
                out.join("native-audio-verification.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            return Ok(());
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-tafsir") {
            let out = PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing tafsir QA output directory")?,
            );
            let source = args
                .get(index + 2)
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(0);
            tafsir_library::network_smoke(&out, source)?;
            return Ok(());
        }
        if let Some(index) = args.iter().position(|s| s == "--verify-network") {
            let out = PathBuf::from(
                args.get(index + 1)
                    .ok_or("Missing audio QA output directory")?,
            );
            std::fs::create_dir_all(&out)?;
            audio::Audio::new()?.network_smoke(&out)?;
            return Ok(());
        }
        if let Some(index) = args
            .iter()
            .position(|s| s == "--verify" || s == "--verify-rendering")
        {
            let out = PathBuf::from(args.get(index + 1).ok_or("Missing QA output directory")?);
            std::fs::create_dir_all(&out)?;
            let mut renderer = render::Renderer::new(&mut resources)?;
            std::fs::write(
                out.join("native-font.txt"),
                renderer.font_family.to_string(),
            )?;
            let mut outline_check =
                outlines::Outlines::new(resources.root.clone(), renderer.factory.clone())?;
            let outline_count = outline_check.verify()?;
            let mut words = std::collections::BTreeMap::new();
            let mut lines = 0;
            for number in 1..=604 {
                let page = resources.page(number)?;
                for line in &page.lines {
                    lines += 1;
                    for word in &line.words {
                        if words.insert(word.id, (word.surah, word.ayah)).is_some() {
                            return Err("Duplicated source word ID".into());
                        }
                    }
                }
            }
            let ayahs = resources.index()?.len();
            let rendering = renderer.verify_layout(&mut resources)?;
            renderer.verify_resized_layout(&mut resources, &out)?;
            std::fs::write(
                out.join("opening-shaping.json"),
                serde_json::to_vec_pretty(&quran_shape::verify(&mut resources)?)?,
            )?;
            for (page, theme, name) in [
                (45, 0, "native-dark-45.png"),
                (535, 0, "native-dark-535.png"),
                (293, 1, "native-light-293.png"),
                (1, 2, "native-sepia-opening.png"),
            ] {
                renderer.snapshot(&mut resources, page, theme, &out.join(name))?;
            }
            renderer.contextual = true;
            renderer.snapshot(&mut resources, 45, 0, &out.join("native-contextual-45.png"))?;
            renderer.snapshot(
                &mut resources,
                503,
                0,
                &out.join("native-contextual-503.png"),
            )?;
            renderer.contextual = false;
            for contextual in [false, true] {
                renderer.contextual = contextual;
                for theme in 0..=3 {
                    let name = format!(
                        "native-highlight-{}-theme-{theme}.png",
                        if contextual { "contextual" } else { "natural" }
                    );
                    renderer.snapshot_case(
                        &mut resources,
                        45,
                        theme,
                        &out.join(name),
                        1.,
                        Some((2, 265)),
                    )?;
                }
                for density in [1.25, 2.] {
                    let name = format!(
                        "native-highlight-{}-dpi-{}.png",
                        if contextual { "contextual" } else { "natural" },
                        (density * 96.) as u32
                    );
                    renderer.snapshot_case(
                        &mut resources,
                        45,
                        0,
                        &out.join(name),
                        density,
                        Some((2, 265)),
                    )?;
                }
            }
            renderer.contextual = false;
            let optional_rendering = renderer.verify_optional_fallback(&resources, &out)?;
            if args[index] == "--verify-rendering" {
                std::fs::write(
                    out.join("native-render-verification.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "passed":true,"pages":604,"ayahs":ayahs,"words":words.len(),"lines":lines,"sourceOutlines":outline_count,
                        "rendering":rendering,"optionalRendering":optional_rendering,"allNativeLayoutsChecked":true,"webviewDependency":false,
                        "scope":"Rendering-only verification. Does not run or certify audio. The existing full --verify command remains strict."
                    }))?,
                )?;
                return Ok(());
            }
            let audio = audio::Audio::new()?.smoke(&out)?;
            let saadi21 = resources.study("saadi", 46, 21)?;
            let saadi22 = resources.study("saadi", 46, 22)?;
            if saadi21.is_empty() || saadi22.is_empty() || saadi21 == saadi22 {
                return Err("Saadi individual section check failed".into());
            }
            let report = serde_json::json!({"pages":604,"ayahs":ayahs,"words":words.len(),"lines":lines,"sourceOutlines":outline_count,"fontFamily":renderer.font_family.to_string(),"audio":audio,"rendering":rendering,"optionalRendering":optional_rendering,"allNativeLayoutsChecked":true,"saadiSectionsChecked":true,"seconds":began.elapsed().as_secs_f64(),"webviewDependency":false});
            std::fs::write(
                out.join("native-verification.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            return Ok(());
        }
    }
    ui::run(resources, began)
}
fn main() {
    if let Err(error) = run() {
        #[cfg(feature = "qa")]
        {
            let args: Vec<_> = std::env::args().collect();
            if let Some(index) = args.iter().position(|s| {
                s == "--verify"
                    || s == "--verify-rendering"
                    || s == "--verify-tafsir"
                    || s == "--verify-audio"
                    || s == "--verify-network"
                    || s == "--verify-timed-store"
                    || s == "--verify-live-rate"
                    || s == "--verify-scoped-store"
                    || s == "--verify-long-timed-audio"
                    || s == "--verify-timed-audio"
                    || s == "--verify-streamed-audio"
                    || s == "--verify-range-stream"
                    || s == "--benchmark-render"
                    || s == "--qa-ui"
                    || s == "--qa-control-colors"
                    || s == "--qa-prepare-scoped-cache"
            }) {
                let output_index = if args[index] == "--verify-streamed-audio" {
                    index + 2
                } else {
                    index + 1
                };
                if let Some(path) = args.get(output_index) {
                    let _ = std::fs::write(
                        {
                            let path = PathBuf::from(path);
                            let _ = std::fs::create_dir_all(&path);
                            path.join("native-error.txt")
                        },
                        error.to_string(),
                    );
                }
                std::process::exit(1);
            }
        }
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(error.to_string()),
                &HSTRING::from("Quran Mushaf Reader — Native Preview"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

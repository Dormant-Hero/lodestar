use super::decode::{fit, load_pixels, thumb_cache_path};
use super::*;
use std::time::Instant;

fn write_png(path: &Path, w: u32, h: u32, alpha: bool) {
    if alpha {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, (x % 200) as u8])
        });
        img.save(path).unwrap();
    } else {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 77]));
        img.save(path).unwrap();
    }
}

fn write_jpeg(path: &Path, w: u32, h: u32) {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 3 % 256) as u8, (y % 256) as u8, 200]));
    img.save_with_format(path, image::ImageFormat::Jpeg).unwrap();
}

fn qoi_count(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "qoi")).count()
}

#[test]
fn fit_never_upscales() {
    assert_eq!(fit(600, 900, 512), (341, 512));
    assert_eq!(fit(300, 200, 512), (300, 200));
    assert_eq!(fit(5000, 10, 512), (512, 1));
    assert_eq!(fit(1, 1, 512), (1, 1));
}

#[test]
fn png_downscale_and_cache_hit() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("thumbs");
    std::fs::create_dir_all(&cache).unwrap();
    let src = tmp.path().join("cover.png");
    write_png(&src, 600, 900, false);

    let a = load_pixels(&cache, &src, 512).unwrap();
    assert_eq!((a.w, a.h, a.channels), (341, 512, 3));
    assert_eq!(qoi_count(&cache), 1, "downscaled image must be cached");
    let cached = thumb_cache_path(&cache, &src, 512).unwrap();
    assert!(cached.exists());

    // Hit path returns identical pixels.
    let b = load_pixels(&cache, &src, 512).unwrap();
    assert_eq!((b.w, b.h, b.channels), (a.w, a.h, a.channels));
    assert_eq!(a.data, b.data);

    // A different size gets its own entry; the medium size of a 600x900 is not downscaled but
    // still worth caching (bigger than SMALL_PIXELS).
    let m = load_pixels(&cache, &src, 1024).unwrap();
    assert_eq!((m.w, m.h), (600, 900));
    assert_eq!(qoi_count(&cache), 2);

    // Changing the file changes the key (size differs) -> fresh decode.
    write_png(&src, 300, 450, false);
    let c = load_pixels(&cache, &src, 512).unwrap();
    assert_eq!((c.w, c.h), (300, 450));
}

#[test]
fn corrupt_cache_file_is_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("c.jpg");
    write_jpeg(&src, 800, 1200);
    let cached = thumb_cache_path(tmp.path(), &src, 512).unwrap();
    std::fs::write(&cached, b"garbage").unwrap();
    let p = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!((p.w, p.h), (341, 512));
    // Rewritten with a valid QOI.
    let bytes = std::fs::read(&cached).unwrap();
    assert_eq!(&bytes[..4], b"qoif");
}

#[test]
fn jpeg_big_ratio_uses_box_prefilter() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("hero.jpg");
    write_jpeg(&src, 3840, 1240);
    let p = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!((p.w, p.h, p.channels), (512, 165, 3));
    assert_eq!(p.data.len(), 512 * 165 * 3);
}

#[test]
fn alpha_png_is_premultiplied_and_small_is_not_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("logo.png");
    write_png(&src, 64, 32, true);
    let p = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!((p.w, p.h, p.channels), (64, 32, 4));
    // Pixel (0,0): alpha 0 -> fully transparent premultiplied.
    assert_eq!(&p.data[..4], &[0, 0, 0, 0]);
    // Pixel (100?,..) - check one with alpha: x=10 -> a=10, r=10 -> premult round(10*10/255)=0
    let i = 4 * 63;
    let (r, a) = (p.data[i], p.data[i + 3]);
    assert_eq!(a, 63);
    assert!(r <= a);
    assert_eq!(qoi_count(tmp.path()), 0, "tiny image needs no cache");
}

#[test]
fn opaque_rgba_becomes_rgb() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("opaque.png");
    image::RgbaImage::from_pixel(40, 40, image::Rgba([1, 2, 3, 255])).save(&src).unwrap();
    let p = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!(p.channels, 3);
    assert_eq!(&p.data[..3], &[1, 2, 3]);
}

#[test]
fn svg_rendered_at_target_size() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("logo.svg");
    std::fs::write(
        &src,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10">
            <rect x="0" y="0" width="10" height="10" fill="#ff0000"/></svg>"##,
    )
    .unwrap();
    let p = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!((p.w, p.h, p.channels), (512, 256, 4));
    // Left half red and opaque, right half transparent.
    assert_eq!(&p.data[..4], &[255, 0, 0, 255]);
    let right = (100 * 512 + 500) * 4;
    assert_eq!(p.data[right + 3], 0);
    assert_eq!(qoi_count(tmp.path()), 1);
    // Cached hit is identical.
    let q = load_pixels(tmp.path(), &src, 512).unwrap();
    assert_eq!(p.data, q.data);
    // Sniffed without extension too.
    let noext = tmp.path().join("logo_noext");
    std::fs::copy(&src, &noext).unwrap();
    assert_eq!(load_pixels(tmp.path(), &noext, 64).unwrap().w, 64);
}

#[test]
fn failures() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(load_pixels(tmp.path(), &tmp.path().join("nope.png"), 512), Err(DecodeError::Missing(_))));
    let bad = tmp.path().join("bad.jpg");
    std::fs::write(&bad, b"\xFF\xD8\xFF\xE0 definitely not a jpeg").unwrap();
    assert!(matches!(load_pixels(tmp.path(), &bad, 512), Err(DecodeError::Decode(_))));
    let txt = tmp.path().join("readme.txt");
    std::fs::write(&txt, b"hello").unwrap();
    assert!(load_pixels(tmp.path(), &txt, 512).is_err());
    let badsvg = tmp.path().join("x.svg");
    std::fs::write(&badsvg, b"<svg broken").unwrap();
    assert!(load_pixels(tmp.path(), &badsvg, 512).is_err());
    assert!(load_pixels(tmp.path(), tmp.path(), 512).is_err(), "directory");
}

#[test]
fn gif_bmp_ico_webp_qoi_decode() {
    let tmp = tempfile::tempdir().unwrap();
    let img = image::RgbaImage::from_fn(48, 48, |x, _| image::Rgba([x as u8 * 5, 10, 20, 255]));
    for (name, fmt) in [
        ("a.gif", image::ImageFormat::Gif),
        ("a.bmp", image::ImageFormat::Bmp),
        ("a.ico", image::ImageFormat::Ico),
        ("a.webp", image::ImageFormat::WebP),
        ("a.qoi", image::ImageFormat::Qoi),
    ] {
        let p = tmp.path().join(name);
        img.save_with_format(&p, fmt).unwrap();
        let px = load_pixels(tmp.path(), &p, 32).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((px.w, px.h), (32, 32), "{name}");
    }
}

#[test]
fn prune_removes_oldest() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..5 {
        std::fs::write(tmp.path().join(format!("{i:016x}.qoi")), vec![0u8; 1000]).unwrap();
        let f = std::fs::File::options().write(true).open(tmp.path().join(format!("{i:016x}.qoi"))).unwrap();
        f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + i)).unwrap();
    }
    std::fs::write(tmp.path().join("other.txt"), b"keep").unwrap();
    let removed = prune_disk_cache(tmp.path(), 3500, 2500);
    assert_eq!(removed, 3);
    assert!(!tmp.path().join(format!("{:016x}.qoi", 0)).exists());
    assert!(tmp.path().join(format!("{:016x}.qoi", 4)).exists());
    assert!(tmp.path().join("other.txt").exists());
}

/// Drive the cache until `path` is ready or failed (headless egui context).
fn pump(cache: &mut ImageCache, path: &Path, size: ImgSize) -> Option<TextureHandle> {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(10) {
        cache.begin_frame();
        if let Some(t) = cache.get(path, size) {
            return Some(t);
        }
        if cache.is_failed(path, size) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timeout waiting for {}", path.display());
}

#[test]
fn cache_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let src = tmp.path().join("c.png");
    write_png(&src, 600, 900, false);
    let missing = tmp.path().join("missing.jpg");

    assert!(cache.get(&src, ImgSize::Thumb).is_none());
    assert!(cache.is_loading(&src, ImgSize::Thumb));
    let t = pump(&mut cache, &src, ImgSize::Thumb).expect("loads");
    assert_eq!(t.size(), [341, 512]);
    assert!(!cache.is_failed(&src, ImgSize::Thumb));

    assert!(pump(&mut cache, &missing, ImgSize::Thumb).is_none());
    assert!(cache.is_failed(&missing, ImgSize::Thumb));
    // Not retried: stays failed without a new job.
    cache.get(&missing, ImgSize::Thumb);
    assert_eq!(cache.stats().queued, 0);
    assert!(cache.is_failed(&missing, ImgSize::Thumb));
    cache.forget_failed();
    assert!(!cache.is_failed(&missing, ImgSize::Thumb));

    let s = cache.stats();
    assert_eq!(s.textures, 1);
    assert_eq!(s.gpu_bytes, 341 * 512 * 4 * 4 / 3);

    cache.invalidate(&src);
    assert_eq!(cache.stats().textures, 0);
    assert!(cache.get(&src, ImgSize::Thumb).is_none());
}

#[test]
fn stale_requests_are_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let mut paths = vec![];
    for i in 0..40 {
        let p = tmp.path().join(format!("{i}.png"));
        write_png(&p, 700, 1000, false);
        paths.push(p);
    }
    // Requests made at frame 100 while the workers already see frame 100 + STALE + 5:
    // as if the user scrolled past them long ago.
    cache.frame = 100;
    cache.shared.frame.store(100 + STALE_FRAMES + 5, Ordering::Relaxed);
    for p in &paths {
        cache.prefetch(p, ImgSize::Thumb);
    }
    let t0 = Instant::now();
    while cache.stats().queued > 0 && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(20));
    cache.frame = 200;
    for _ in 0..3 {
        cache.begin_frame();
    }
    let s = cache.stats();
    assert_eq!((s.textures, s.pending, s.failed), (0, 0, 0), "stale jobs must be dropped undecoded: {s:?}");
    assert_eq!(qoi_count(&tmp.path().join("thumbs")), 0, "nothing decoded");
    // A dropped request is simply re-scheduled when asked again.
    assert!(pump(&mut cache, &paths[3], ImgSize::Thumb).is_some());
}

#[test]
fn lru_eviction_respects_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    // Each 100x100 thumb = 40000 * 4/3 = 53333 bytes; budget for ~3.
    let mut cache = ImageCache::with_budget(&ctx, tmp.path().join("thumbs"), 170_000);
    let mut paths = vec![];
    for i in 0..6 {
        let p = tmp.path().join(format!("{i}.png"));
        write_png(&p, 100, 100, false);
        paths.push(p);
    }
    for p in &paths {
        let _ = pump(&mut cache, p, ImgSize::Thumb).unwrap();
        cache.begin_frame();
        cache.begin_frame();
    }
    for _ in 0..3 {
        cache.begin_frame();
    }
    let s = cache.stats();
    assert!(s.gpu_bytes <= 170_000, "{s:?}");
    assert!(s.evicted_total >= 3, "{s:?}");
    // The most recent survives, the first was evicted.
    assert!(cache.get(paths.last().unwrap(), ImgSize::Thumb).is_some());
    assert!(cache.get(&paths[0], ImgSize::Thumb).is_none());
}

#[test]
fn upload_budget_per_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let mut paths = vec![];
    for i in 0..20 {
        let p = tmp.path().join(format!("{i}.png"));
        write_png(&p, 64, 64, false);
        paths.push(p);
    }
    for p in &paths {
        cache.get(p, ImgSize::Thumb);
    }
    let t0 = Instant::now();
    while cache.stats().queued > 0 && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(20));
    cache.begin_frame();
    assert!(cache.stats().uploads_last_frame <= MAX_UPLOADS_PER_FRAME);
    let mut frames = 1;
    while cache.stats().textures < 20 && frames < 50 {
        cache.begin_frame();
        frames += 1;
    }
    assert_eq!(cache.stats().textures, 20);
    assert!(frames >= 20 / MAX_UPLOADS_PER_FRAME, "{frames}");
}

#[test]
fn get_hit_path_is_cheap() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let src = tmp.path().join("c.png");
    write_png(&src, 64, 64, false);
    let _ = pump(&mut cache, &src, ImgSize::Thumb).unwrap();
    let t0 = Instant::now();
    let n = 100_000;
    for _ in 0..n {
        std::hint::black_box(cache.get(&src, ImgSize::Thumb));
    }
    let per = t0.elapsed() / n;
    eprintln!("get() hit: {per:?}");
    assert!(per < Duration::from_micros(20));
}

/// Real-data benchmark (run with `cargo test images::tests::bench -- --ignored --nocapture`).
#[test]
#[ignore]
fn bench_real_steam_covers() {
    let Some(home) = dirs::home_dir() else { return };
    let lc = home.join(".local/share/Steam/appcache/librarycache");
    let Ok(rd) = std::fs::read_dir(&lc) else {
        eprintln!("no steam library cache");
        return;
    };
    let covers: Vec<PathBuf> =
        rd.flatten().map(|e| e.path().join("library_600x900.jpg")).filter(|p| p.exists()).take(50).collect();
    let heroes: Vec<PathBuf> = std::fs::read_dir(&lc)
        .unwrap()
        .flatten()
        .map(|e| e.path().join("library_hero.jpg"))
        .filter(|p| p.exists())
        .take(10)
        .collect();
    eprintln!("{} covers, {} heroes", covers.len(), heroes.len());
    let tmp = tempfile::tempdir().unwrap();
    let run = |label: &str, set: &[PathBuf], px: u32| {
        let t0 = Instant::now();
        for p in set {
            load_image(tmp.path(), p, px).unwrap();
        }
        let cold = t0.elapsed() / set.len().max(1) as u32;
        let t0 = Instant::now();
        for p in set {
            load_image(tmp.path(), p, px).unwrap();
        }
        let warm = t0.elapsed() / set.len().max(1) as u32;
        eprintln!("{label} @{px}: cold {cold:?}/img, cached {warm:?}/img");
    };
    // Raw decode only, for reference.
    if let Some(p) = covers.first() {
        let t0 = Instant::now();
        for _ in 0..10 {
            // Note: some Steam "library_600x900.jpg" files are really PNGs; sniff the format.
            image::ImageReader::open(p).unwrap().with_guessed_format().unwrap().decode().unwrap();
        }
        eprintln!("image::open jpeg 600x900: {:?}", t0.elapsed() / 10);
    }
    run("cover", &covers, 512);
    run("cover", &covers, 1024);
    run("hero", &heroes, 2560);
    run("hero", &heroes, 1024);
    let sz: u64 =
        std::fs::read_dir(tmp.path()).unwrap().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
    eprintln!("cache dir size {} KB", sz / 1024);
}

/// After a fast scroll, results for rows no longer on screen are dropped instead of uploaded,
/// so the rows now visible don't wait behind them (uploads are capped per frame).
#[test]
fn results_scrolled_past_are_not_uploaded() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let paths: Vec<PathBuf> = (0..20)
        .map(|i| {
            let p = tmp.path().join(format!("r{i}.png"));
            write_png(&p, 40, 60, false);
            p
        })
        .collect();
    // Frame 1: twenty rows were on screen for one frame only…
    cache.begin_frame();
    for p in &paths {
        assert!(cache.get(p, ImgSize::Icon).is_none());
    }
    // … then scrolled away: wait for the decodes while frames pass without asking again.
    let t0 = Instant::now();
    while cache.shared.busy.load(Ordering::Relaxed) > 0
        || cache.shared.stack.lock().len() > 0
        || cache.done_rx.len() < 20
    {
        assert!(t0.elapsed() < Duration::from_secs(10), "decodes never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    cache.frame += UPLOAD_STALE_FRAMES + 1;
    // The row on screen now arrives in the same frame although twenty results were ahead of it.
    let now_visible = tmp.path().join("visible.png");
    write_png(&now_visible, 40, 60, false);
    assert!(cache.get(&now_visible, ImgSize::Icon).is_none());
    let tex = pump(&mut cache, &now_visible, ImgSize::Icon);
    assert!(tex.is_some());
    let s = cache.stats();
    assert_eq!(s.textures, 1, "only the visible row was uploaded: {s:?}");
    assert_eq!(s.pending, 0, "the dropped rows are forgotten (they load again when shown)");
    // A row scrolled back into view loads again.
    assert!(pump(&mut cache, &paths[3], ImgSize::Icon).is_some());
}

/// Pictures on screen are decoded before prefetched ones; a prefetched picture that comes on
/// screen while still queued is moved up and decoded once; `waiting` counts what is on screen
/// and still loading.
#[test]
fn on_screen_pictures_go_before_prefetches() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut cache = ImageCache::new(&ctx, tmp.path().join("thumbs"));
    let mk = |name: &str| {
        let p = tmp.path().join(name);
        write_png(&p, 40, 60, false);
        p
    };
    let (pre, pre2, shown) = (mk("pre.png"), mk("pre2.png"), mk("shown.png"));
    let mut q = Queues::default();
    let job = |p: &Path| Job {
        key: (path_hash(p), ImgSize::Icon, Adjust::NONE),
        path: p.to_path_buf(),
        size: ImgSize::Icon,
        adjust: Adjust::NONE,
        last_req: Arc::new(AtomicU64::new(0)),
        taken: None,
    };
    q.low.push(job(&pre));
    q.high.push(job(&shown));
    q.low.push(job(&pre2));
    assert_eq!(q.pop().unwrap().path, shown, "on screen first");
    assert_eq!(q.pop().unwrap().path, pre2, "then the latest prefetch");
    assert_eq!(q.len(), 1);

    // Through the cache: prefetch, then show it; it loads exactly once.
    cache.prefetch(&pre, ImgSize::Icon);
    assert_eq!(cache.waiting(), 0, "a prefetch isn't on screen");
    // Shown while (perhaps) still queued as a prefetch: moved up, decoded once.
    let _ = cache.get(&pre, ImgSize::Icon);
    let tex = pump(&mut cache, &pre, ImgSize::Icon);
    assert!(tex.is_some());
    assert_eq!(cache.stats().textures, 1);
    assert_eq!(cache.waiting(), 0);
    // Shown and still loading: counted.
    let late = mk("late.png");
    cache.begin_frame();
    assert!(cache.get(&late, ImgSize::Icon).is_none());
    assert_eq!(cache.waiting(), 1);
}

#[test]
fn colour_adjustments_follow_es_de() {
    let px = |c: egui::Color32| ColorImage::new([1, 1], vec![c]);
    // Saturation 0: grey of the same luminance.
    let mut img = px(egui::Color32::from_rgb(200, 40, 40));
    Adjust::new(0.0, 0.0).apply(&mut img);
    let [r, g, b, a] = img.pixels[0].to_array();
    assert!(r == g && g == b && a == 255, "{r} {g} {b}");
    assert!((r as i32 - 74).abs() <= 1, "{r}");
    // Brightness -1.5 takes 0.45 off every channel; +2 turns a logo white.
    let mut img = px(egui::Color32::from_rgb(255, 128, 0));
    Adjust::new(-1.5, 1.0).apply(&mut img);
    assert_eq!(img.pixels[0].to_array(), [140, 13, 0, 255]);
    let mut img = px(egui::Color32::from_rgb(110, 110, 110));
    Adjust::new(2.0, 1.0).apply(&mut img);
    assert_eq!(img.pixels[0].to_array(), [255, 255, 255, 255]);
    // On the colour, not the premultiplied value: half-transparent stays half-transparent.
    let mut img = px(egui::Color32::from_rgba_premultiplied(50, 50, 50, 128));
    Adjust::new(1.0, 1.0).apply(&mut img);
    let [r, _, _, a] = img.pixels[0].to_array();
    assert_eq!(a, 128);
    assert!((r as i32 - 88).abs() <= 1, "{r}");
    // Neutral settings key the plain texture.
    assert!(Adjust::new(0.0, 1.0).is_none());
    assert!(!Adjust::new(0.0, 0.99).is_none());
}

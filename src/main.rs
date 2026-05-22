#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use eframe::egui;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::Command,
};

const APP_VERSION: &str = "1.3.0";
const STEAM_APP_ID: &str = "219780";
const DATA_RELATIVE: &[&str] = &["Data", "Win32", "Packed", "MainDataStreaming.dv2"];
const OFFSET: usize = 0x53963;
const PACKED_SIZE: usize = 0x5FB3;
const EXPECTED_UNPACKED_SIZE: usize = 181_669;
const PLACEHOLDER: &str = "DO NOT TRANSLATE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    DryRun,
    PatchWrite,
}

#[derive(Debug, Clone)]
struct RunReport {
    ok: bool,
    message: String,
    log: String,
    log_path: Option<PathBuf>,
    backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct PatchSet {
    name: &'static str,
    entries: Vec<PatchEntry>,
}

#[derive(Debug, Clone)]
struct PatchEntry {
    english: &'static str,
    replacement16: &'static str,
}

#[derive(Debug, Clone)]
struct CompressionCandidate {
    name: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BestCandidate {
    patch_set_name: String,
    compression_name: String,
    bytes: Vec<u8>,
    patched_dec: Vec<u8>,
    total_patched: usize,
}

#[derive(Default)]
struct GuiApp {
    game_root: String,
    allow_compact_names: bool,
    log: String,
    last_log_path: Option<PathBuf>,
    last_backup_path: Option<PathBuf>,
    status: String,
    status_ok: Option<bool>,
}

impl GuiApp {
    fn new() -> Self {
        let mut app = Self {
            allow_compact_names: true,
            ..Default::default()
        };
        app.append_gui("Start aplikacji. Gotowy fix dla polskiej wersji gry.");
        match detect_game_root() {
            Some(p) => {
                app.game_root = p.to_string_lossy().to_string();
                app.status = "Gra wykryta automatycznie.".to_string();
                app.status_ok = Some(true);
                app.append_gui(&format!("Automatycznie wykryto grę: {}", app.game_root));
            }
            None => {
                app.status = "Nie wykryto gry automatycznie. Wskaż folder ręcznie.".to_string();
                app.status_ok = None;
                app.append_gui("Nie udało się automatycznie wykryć lokalizacji gry.");
            }
        }
        app
    }

    fn append_gui(&mut self, text: &str) {
        self.log.push_str(&format!("[{}] {}\n", timestamp_hms(), text));
    }

    fn apply_report(&mut self, report: RunReport) {
        self.log.push_str(&report.log);
        self.status = report.message.clone();
        self.status_ok = Some(report.ok);
        self.last_log_path = report.log_path.clone();
        self.last_backup_path = report.backup_path.clone();
    }

    fn run_checked(&mut self, mode: RunMode) {
        let root = PathBuf::from(self.game_root.trim());
        let report = run_patch_pipeline(&root, mode, self.allow_compact_names);
        self.apply_report(report);
    }

    fn run_restore(&mut self) {
        let root = PathBuf::from(self.game_root.trim());
        let report = restore_latest_backup(&root);
        self.apply_report(report);
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Divinity II PL DO NOT TRANSLATE Fix");
                ui.label("v1.3");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);

            let status_color = match self.status_ok {
                Some(true) => egui::Color32::from_rgb(30, 140, 70),
                Some(false) => egui::Color32::from_rgb(180, 45, 45),
                None => ui.visuals().text_color(),
            };
            ui.colored_label(status_color, &self.status);

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label("Folder gry:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.game_root);
                    if ui.button("Wykryj automatycznie").clicked() {
                        match detect_game_root() {
                            Some(p) => {
                                self.game_root = p.to_string_lossy().to_string();
                                self.status = "Gra wykryta automatycznie.".to_string();
                                self.status_ok = Some(true);
                                self.append_gui(&format!("Automatycznie wykryto grę: {}", self.game_root));
                            }
                            None => {
                                self.status = "Nie udało się wykryć gry automatycznie.".to_string();
                                self.status_ok = Some(false);
                                self.append_gui("Automatyczne wykrywanie nie znalazło gry.");
                            }
                        }
                    }
                    if ui.button("Wskaż ręcznie").clicked() {
                        if let Some(folder) = rfd::FileDialog::new().set_title("Wybierz folder divinity2_dev_cut").pick_folder() {
                            self.game_root = folder.to_string_lossy().to_string();
                            self.status = "Folder ustawiony ręcznie.".to_string();
                            self.status_ok = None;
                            self.append_gui(&format!("Wybrano folder ręcznie: {}", self.game_root));
                        }
                    }
                });

                ui.checkbox(
                    &mut self.allow_compact_names,
                    "Awaryjnie zezwól na krótsze/kompresowalne nazwy, tylko jeśli standardowe tłumaczenia nie mieszczą się w slocie",
                );
            });

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("1. Sprawdź bez zapisu").clicked() {
                    self.run_checked(RunMode::DryRun);
                }
                if ui.button("2. Zrób backup i patchuj").clicked() {
                    self.run_checked(RunMode::PatchWrite);
                }
                if ui.button("Przywróć najnowszy backup").clicked() {
                    self.run_restore();
                }
                if ui.button("Otwórz log").clicked() {
                    if let Some(path) = &self.last_log_path {
                        open_file(path);
                    } else {
                        self.append_gui("Brak logu do otwarcia.");
                    }
                }
                if ui.button("Otwórz folder backupów").clicked() {
                    let root = PathBuf::from(self.game_root.trim());
                    let backup_dir = root.join("_dv2_localisation_patcher_backups");
                    open_folder(&backup_dir);
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.label("Log:");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(24)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Divinity II PL DO NOT TRANSLATE Fix",
        options,
        Box::new(|_cc| Ok(Box::new(GuiApp::new()))),
    )
}

fn run_patch_pipeline(game_root: &Path, mode: RunMode, allow_compact_names: bool) -> RunReport {
    let mut log = String::new();
    let mut log_path = None;
    let mut backup_path = None;

    let result = (|| -> Result<String> {
        log_line(&mut log, &format!("DV2 localisation patcher {}", APP_VERSION));
        log_line(&mut log, &format!("Mode: {:?}", mode));
        log_line(&mut log, &format!("Game root: {}", game_root.display()));

        let data_path = data_file_path(game_root);
        log_line(&mut log, &format!("Data file: {}", data_path.display()));
        if !data_path.exists() {
            bail!("Nie znaleziono pliku gry: {}", data_path.display());
        }

        ensure_dirs(game_root)?;
        log_line(&mut log, &format!("Offset: 0x{:X}", OFFSET));
        log_line(&mut log, &format!("Packed size: {}", PACKED_SIZE));

        let whole = fs::read(&data_path).with_context(|| format!("Nie mogę odczytać {}", data_path.display()))?;
        if whole.len() < OFFSET + PACKED_SIZE {
            bail!("Plik jest zbyt mały dla oczekiwanego offsetu i rozmiaru slotu.");
        }

        let raw = &whole[OFFSET..OFFSET + PACKED_SIZE];
        let dec = decompress_zlib(raw).context("Nie udało się zdekompresować bloku zlib z DV2")?;
        log_line(&mut log, &format!("Decompressed size: {}", dec.len()));
        if dec.len() != EXPECTED_UNPACKED_SIZE {
            bail!(
                "Nieoczekiwany rozmiar po dekompresji: {}, oczekiwano {}. To może być inna wersja pliku.",
                dec.len(),
                EXPECTED_UNPACKED_SIZE
            );
        }

        log_line(&mut log, "");
        log_line(&mut log, "===== BEFORE CHECK =====");
        let before_remaining = count_target_dnt(&dec, &mut log);
        log_line(&mut log, &format!("Target nearby DNT count before patch: {}", before_remaining));

        if before_remaining == 0 {
            if looks_already_patched(&dec) {
                return Ok("Wygląda na to, że plik jest już spatchowany — nie wykonano zapisu.".to_string());
            }
            bail!("Nie znaleziono docelowych placeholderów DO NOT TRANSLATE. Plik może być już zmieniony albo pochodzić z innej wersji gry.");
        }

        let patch_sets = patch_sets(allow_compact_names);
        let best = find_best_patch_and_compression(&dec, &patch_sets, &mut log)?;

        log_line(&mut log, "");
        log_line(&mut log, "===== SELECTED RESULT =====");
        log_line(&mut log, &format!("Patch set: {}", best.patch_set_name));
        log_line(&mut log, &format!("Compression: {}", best.compression_name));
        log_line(&mut log, &format!("Compressed size: {} / {}", best.bytes.len(), PACKED_SIZE));
        log_line(&mut log, &format!("Total patched in memory: {}", best.total_patched));

        if mode == RunMode::DryRun {
            return Ok(format!(
                "Test OK. Wybrano: {} + {}, rozmiar {} / {}. Zapis nie został wykonany.",
                best.patch_set_name,
                best.compression_name,
                best.bytes.len(),
                PACKED_SIZE
            ));
        }

        let backup = create_backup(&data_path, game_root, "before-patch-rust-zopfli")?;
        log_line(&mut log, &format!("Backup created: {}", backup.display()));
        backup_path = Some(backup);

        let mut new_whole = whole.clone();
        for b in &mut new_whole[OFFSET..OFFSET + PACKED_SIZE] {
            *b = 0;
        }
        new_whole[OFFSET..OFFSET + best.bytes.len()].copy_from_slice(&best.bytes);
        fs::write(&data_path, &new_whole).with_context(|| format!("Nie mogę zapisać {}", data_path.display()))?;
        log_line(&mut log, "Patched file written.");

        log_line(&mut log, "");
        log_line(&mut log, "===== VERIFY FROM DISK =====");
        let verify_whole = fs::read(&data_path)?;
        let verify_raw = &verify_whole[OFFSET..OFFSET + PACKED_SIZE];
        let verify_dec = decompress_zlib(verify_raw).context("Weryfikacja z dysku: nie udało się zdekompresować zapisanego strumienia")?;
        log_line(&mut log, &format!("Verify decompressed size: {}", verify_dec.len()));
        if verify_dec != best.patched_dec {
            bail!("Weryfikacja z dysku nie zgadza się z danymi patchowanymi w pamięci.");
        }
        let remaining_after = count_target_dnt(&verify_dec, &mut log);
        log_line(&mut log, &format!("Remaining nearby DNT after disk write: {}", remaining_after));
        if remaining_after != 0 {
            bail!("Po zapisie dalej istnieją docelowe DO NOT TRANSLATE. Przerywam.");
        }

        Ok(format!(
            "Patch OK. Metoda: {} + {}. Backup: {}",
            best.patch_set_name,
            best.compression_name,
            backup_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
        ))
    })();

    let ok;
    let message;
    match result {
        Ok(msg) => {
            ok = true;
            message = msg;
            log_line(&mut log, "DONE");
        }
        Err(err) => {
            ok = false;
            message = format!("Błąd: {err:#}");
            log_line(&mut log, &message);
        }
    }

    if game_root.exists() {
        if let Ok(path) = save_log(game_root, &log) {
            log_path = Some(path);
        }
    }

    RunReport {
        ok,
        message,
        log,
        log_path,
        backup_path,
    }
}

fn find_best_patch_and_compression(dec: &[u8], patch_sets: &[PatchSet], log: &mut String) -> Result<BestCandidate> {
    let mut best_too_large: Option<(String, String, usize)> = None;

    for patch_set in patch_sets {
        log_line(log, "");
        log_line(log, &format!("===== TRY PATCH SET: {} =====", patch_set.name));
        let mut patched_dec = dec.to_vec();
        let mut already = HashSet::new();
        let mut total = 0usize;

        for entry in &patch_set.entries {
            total += patch_near(
                &mut patched_dec,
                entry.english,
                entry.replacement16,
                &mut already,
                log,
            )?;
        }

        log_line(log, &format!("Total patched for set '{}': {}", patch_set.name, total));
        if total < 3 {
            log_line(log, "Patch set rejected: expected at least 3 patches.");
            continue;
        }

        log_line(log, "");
        log_line(log, "===== AFTER CHECK BEFORE COMPRESSION =====");
        let remaining = count_target_dnt(&patched_dec, log);
        log_line(log, &format!("Remaining nearby DNT for target names: {}", remaining));
        if remaining != 0 {
            log_line(log, "Patch set rejected: target DNT still present.");
            continue;
        }

        let candidates = compression_candidates(&patched_dec, log)?;
        for candidate in candidates {
            let size = candidate.bytes.len();
            verify_compressed_candidate(&candidate, &patched_dec, log)?;
            log_line(log, &format!("Candidate {}: {} bytes", candidate.name, size));

            if size <= PACKED_SIZE {
                log_line(log, &format!("Candidate fits in slot: {} <= {}", size, PACKED_SIZE));
                return Ok(BestCandidate {
                    patch_set_name: patch_set.name.to_string(),
                    compression_name: candidate.name,
                    bytes: candidate.bytes,
                    patched_dec,
                    total_patched: total,
                });
            }

            match &best_too_large {
                Some((_ps, _cn, best_size)) if *best_size <= size => {}
                _ => best_too_large = Some((patch_set.name.to_string(), candidate.name.clone(), size)),
            }
        }
    }

    if let Some((ps, cn, size)) = best_too_large {
        bail!(
            "Żadna metoda nie zmieściła się w slocie. Najbliżej było: patch set '{}', kompresja '{}', {} bajtów, czyli o {} bajtów za dużo.",
            ps,
            cn,
            size,
            size.saturating_sub(PACKED_SIZE)
        );
    }

    bail!("Nie udało się przygotować poprawnego patcha.");
}

fn compression_candidates(data: &[u8], log: &mut String) -> Result<Vec<CompressionCandidate>> {
    let mut candidates = Vec::new();

    log_line(log, "");
    log_line(log, "===== COMPRESSION CANDIDATES =====");

    // flate2/miniz path: quick candidates, useful for diagnostics and sometimes enough.
    for level in (1u32..=9u32).rev() {
        let bytes = compress_flate2_zlib(data, level)?;
        candidates.push(CompressionCandidate {
            name: format!("flate2-zlib-level-{}", level),
            bytes,
        });
    }

    // Zopfli path: slower but usually much denser. We try several safe option sets.
    let zopfli_option_sets: &[(u64, u16, &str)] = &[
        (5, 15, "zopfli-iter5-splits15"),
        (10, 15, "zopfli-iter10-splits15"),
        (15, 15, "zopfli-iter15-splits15-defaultish"),
        (25, 15, "zopfli-iter25-splits15"),
        (50, 15, "zopfli-iter50-splits15"),
        (15, 0, "zopfli-iter15-splits-unlimited"),
        (25, 0, "zopfli-iter25-splits-unlimited"),
        (50, 0, "zopfli-iter50-splits-unlimited"),
    ];

    for (iterations, splits, name) in zopfli_option_sets {
        let bytes = compress_zopfli_zlib(data, *iterations, *splits)
            .with_context(|| format!("Zopfli candidate '{}' failed", name))?;
        candidates.push(CompressionCandidate {
            name: (*name).to_string(),
            bytes,
        });
    }

    candidates.sort_by_key(|c| c.bytes.len());
    Ok(candidates)
}

fn compress_flate2_zlib(data: &[u8], level: u32) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn compress_zopfli_zlib(data: &[u8], iterations: u64, maximum_block_splits: u16) -> Result<Vec<u8>> {
    let mut options = zopfli::Options::default();
    options.iteration_count = NonZeroU64::new(iterations).ok_or_else(|| anyhow!("iterations must be non-zero"))?;
    options.maximum_block_splits = maximum_block_splits;

    let mut out = Vec::new();
    zopfli::compress(options, zopfli::Format::Zlib, Cursor::new(data), &mut out)
        .map_err(|e| anyhow!("Zopfli error: {e:?}"))?;
    Ok(out)
}

fn verify_compressed_candidate(candidate: &CompressionCandidate, expected_dec: &[u8], log: &mut String) -> Result<()> {
    let roundtrip = decompress_zlib(&candidate.bytes)
        .with_context(|| format!("Candidate '{}' does not decompress as zlib", candidate.name))?;
    if roundtrip != expected_dec {
        bail!("Candidate '{}' roundtrip mismatch", candidate.name);
    }
    log_line(log, &format!("Candidate verified: {}", candidate.name));
    Ok(())
}

fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(Cursor::new(data));
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn patch_sets(allow_compact_names: bool) -> Vec<PatchSet> {
    let mut sets = vec![PatchSet {
        name: "standard-polish-labels",
        entries: vec![
            PatchEntry { english: "Malachite Ore", replacement16: "Ruda malachitu  " },
            PatchEntry { english: "Gold Ore", replacement16: "Ruda zlota      " },
            PatchEntry { english: "Malachite Gem", replacement16: "Malachit        " },
            PatchEntry { english: "Sapphire", replacement16: "Szafir          " },
            PatchEntry { english: "Spinel", replacement16: "Spinel          " },
        ],
    }];

    if allow_compact_names {
        sets.push(PatchSet {
            name: "fallback-compression-friendly-polish-labels",
            entries: vec![
                PatchEntry { english: "Malachite Ore", replacement16: "Ruda malachit   " },
                PatchEntry { english: "Gold Ore", replacement16: "Ruda zlota      " },
                PatchEntry { english: "Malachite Gem", replacement16: "Malachit        " },
                PatchEntry { english: "Sapphire", replacement16: "Szafir          " },
                PatchEntry { english: "Spinel", replacement16: "Spinel          " },
            ],
        });
        sets.push(PatchSet {
            name: "fallback-ultra-compact-polish-labels",
            entries: vec![
                PatchEntry { english: "Malachite Ore", replacement16: "Malachit ruda   " },
                PatchEntry { english: "Gold Ore", replacement16: "Zloto           " },
                PatchEntry { english: "Malachite Gem", replacement16: "Malachit        " },
                PatchEntry { english: "Sapphire", replacement16: "Szafir          " },
                PatchEntry { english: "Spinel", replacement16: "Spinel          " },
            ],
        });
    }

    sets
}

fn patch_near(
    data: &mut [u8],
    english_name: &str,
    replacement16: &str,
    already_patched: &mut HashSet<usize>,
    log: &mut String,
) -> Result<usize> {
    if !replacement16.is_ascii() || replacement16.len() != 16 {
        bail!("Replacement must be exactly 16 ASCII chars: '{}', len={}", replacement16, replacement16.len());
    }

    let english_positions = find_all(data, english_name.as_bytes());
    let placeholder_positions = find_all(data, PLACEHOLDER.as_bytes());
    log_line(log, &format!("English: {} | occurrences: {}", english_name, english_positions.len()));

    let repl = replacement16.as_bytes();
    let mut patched = 0usize;

    for eng in english_positions {
        let mut best: Option<usize> = None;
        let mut best_distance = usize::MAX;

        for &ph in &placeholder_positions {
            if eng < ph {
                continue;
            }
            let distance = eng - ph;
            if (16..=80).contains(&distance) && !already_patched.contains(&ph) && distance < best_distance {
                best = Some(ph);
                best_distance = distance;
            }
        }

        if let Some(pos) = best {
            log_line(log, &format!("  PATCH at 0x{:X} before '{}' distance={}", pos, english_name, best_distance));
            log_line(log, &format!("  BEFORE: {}", context(data, pos)));
            data[pos..pos + repl.len()].copy_from_slice(repl);
            already_patched.insert(pos);
            patched += 1;
            log_line(log, &format!("  AFTER : {}", context(data, pos)));
        } else {
            log_line(log, &format!("  no nearby placeholder before occurrence at 0x{:X}", eng));
        }
    }

    log_line(log, &format!("  patched count for {}: {}", english_name, patched));
    Ok(patched)
}

fn count_target_dnt(data: &[u8], log: &mut String) -> usize {
    let mut count = 0usize;
    count += count_nearby_dnt(data, "Malachite Ore", log);
    count += count_nearby_dnt(data, "Gold Ore", log);
    count += count_nearby_dnt(data, "Malachite Gem", log);
    count
}

fn count_nearby_dnt(data: &[u8], english_name: &str, log: &mut String) -> usize {
    let english_positions = find_all(data, english_name.as_bytes());
    let placeholder_positions = find_all(data, PLACEHOLDER.as_bytes());
    let mut count = 0usize;

    for eng in english_positions {
        for &ph in &placeholder_positions {
            if eng >= ph {
                let distance = eng - ph;
                if (16..=80).contains(&distance) {
                    count += 1;
                    log_line(log, &format!("  remaining DNT near '{}' at placeholder 0x{:X}, english 0x{:X}, distance={}", english_name, ph, eng, distance));
                    log_line(log, &format!("  context: {}", context(data, ph)));
                }
            }
        }
    }

    count
}

fn looks_already_patched(data: &[u8]) -> bool {
    find_all(data, b"Ruda malachitu  ").len() >= 1
        || find_all(data, b"Ruda malachit   ").len() >= 1
        || find_all(data, b"Malachit ruda   ").len() >= 1
}

fn find_all(data: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > data.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..=data.len() - needle.len() {
        if &data[i..i + needle.len()] == needle {
            out.push(i);
        }
    }
    out
}

fn context(data: &[u8], pos: usize) -> String {
    let start = pos.saturating_sub(80);
    let end = (start + 220).min(data.len());
    data[start..end]
        .iter()
        .map(|&b| if (32..=126).contains(&b) { b as char } else { ' ' })
        .collect()
}

fn create_backup(data_path: &Path, game_root: &Path, tag: &str) -> Result<PathBuf> {
    let backup_dir = game_root.join("_dv2_localisation_patcher_backups");
    fs::create_dir_all(&backup_dir)?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S-%3f");
    let backup_path = backup_dir.join(format!("MainDataStreaming.dv2.{}.{}.bak", tag, stamp));
    fs::copy(data_path, &backup_path)?;
    Ok(backup_path)
}

fn restore_latest_backup(game_root: &Path) -> RunReport {
    let mut log = String::new();
    let mut log_path = None;
    let mut backup_path = None;

    let result = (|| -> Result<String> {
        log_line(&mut log, &format!("DV2 localisation patcher {}", APP_VERSION));
        log_line(&mut log, "Mode: restore latest backup");
        log_line(&mut log, &format!("Game root: {}", game_root.display()));

        let data_path = data_file_path(game_root);
        let backup_dir = game_root.join("_dv2_localisation_patcher_backups");
        if !backup_dir.exists() {
            bail!("Brak folderu backupów: {}", backup_dir.display());
        }

        let mut backups: Vec<PathBuf> = fs::read_dir(&backup_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("MainDataStreaming.dv2.") && n.ends_with(".bak"))
                    .unwrap_or(false)
            })
            .collect();

        backups.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
        let latest = backups.pop().ok_or_else(|| anyhow!("Nie znaleziono żadnego backupu."))?;
        log_line(&mut log, &format!("Latest backup: {}", latest.display()));

        if data_path.exists() {
            let safety = create_backup(&data_path, game_root, "before-restore-current")?;
            log_line(&mut log, &format!("Current file safety backup: {}", safety.display()));
            backup_path = Some(safety);
        }

        fs::copy(&latest, &data_path)?;
        log_line(&mut log, &format!("Restored to: {}", data_path.display()));

        // Minimal validation: the restored file should still contain a decompressable target slot.
        let whole = fs::read(&data_path)?;
        if whole.len() >= OFFSET + PACKED_SIZE {
            let raw = &whole[OFFSET..OFFSET + PACKED_SIZE];
            let dec = decompress_zlib(raw)?;
            log_line(&mut log, &format!("Restored slot decompressed size: {}", dec.len()));
        }

        Ok(format!("Przywrócono backup: {}", latest.display()))
    })();

    let ok;
    let message;
    match result {
        Ok(msg) => {
            ok = true;
            message = msg;
            log_line(&mut log, "DONE");
        }
        Err(err) => {
            ok = false;
            message = format!("Błąd: {err:#}");
            log_line(&mut log, &message);
        }
    }

    if game_root.exists() {
        if let Ok(path) = save_log(game_root, &log) {
            log_path = Some(path);
        }
    }

    RunReport { ok, message, log, log_path, backup_path }
}

fn ensure_dirs(game_root: &Path) -> Result<()> {
    fs::create_dir_all(game_root.join("_dv2_localisation_patcher_backups"))?;
    fs::create_dir_all(game_root.join("_dv2_localisation_patcher_logs"))?;
    Ok(())
}

fn save_log(game_root: &Path, log: &str) -> Result<PathBuf> {
    let logs_dir = game_root.join("_dv2_localisation_patcher_logs");
    fs::create_dir_all(&logs_dir)?;
    let path = logs_dir.join(format!("rust-zopfli-patcher-{}.txt", Local::now().format("%Y%m%d-%H%M%S-%3f")));
    fs::write(&path, log)?;
    Ok(path)
}

fn log_line(log: &mut String, text: &str) {
    log.push_str(&format!("[{}] {}\n", timestamp_hms(), text));
}

fn timestamp_hms() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn data_file_path(game_root: &Path) -> PathBuf {
    let mut p = game_root.to_path_buf();
    for part in DATA_RELATIVE {
        p.push(part);
    }
    p
}

fn detect_game_root() -> Option<PathBuf> {
    let mut steam_roots = detect_steam_roots();
    steam_roots.sort();
    steam_roots.dedup();

    let mut libraries = Vec::new();
    for root in &steam_roots {
        libraries.push(root.clone());
        libraries.extend(parse_libraryfolders(root));
    }
    libraries.sort();
    libraries.dedup();

    for lib in libraries {
        if let Some(root) = find_game_in_steam_library(&lib) {
            return Some(root);
        }
    }

    None
}

fn detect_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam") {
            if let Ok(path_str) = key.get_value::<String, _>("SteamPath") {
                roots.push(PathBuf::from(path_str));
            }
            if let Ok(path_str) = key.get_value::<String, _>("SteamExe") {
                if let Some(parent) = PathBuf::from(path_str).parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }
    }

    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        roots.push(PathBuf::from(program_files_x86).join("Steam"));
    }
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        roots.push(PathBuf::from(program_files).join("Steam"));
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    roots.push(PathBuf::from(r"E:\Steam"));
    roots.push(PathBuf::from(r"D:\Steam"));

    roots.into_iter().filter(|p| p.exists()).collect()
}

fn parse_libraryfolders(steam_root: &Path) -> Vec<PathBuf> {
    let path = steam_root.join("steamapps").join("libraryfolders.vdf");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("\"path\"") {
            continue;
        }
        let quoted: Vec<&str> = trimmed.split('"').collect();
        if quoted.len() >= 4 {
            let raw_path = quoted[3].replace("\\\\", "\\");
            let p = PathBuf::from(raw_path);
            if p.exists() {
                out.push(p);
            }
        }
    }
    out
}

fn find_game_in_steam_library(library_root: &Path) -> Option<PathBuf> {
    let steamapps = library_root.join("steamapps");
    let common = steamapps.join("common");

    // Prefer manifest-based detection.
    let manifest = steamapps.join(format!("appmanifest_{}.acf", STEAM_APP_ID));
    if manifest.exists() {
        if let Ok(text) = fs::read_to_string(&manifest) {
            if let Some(installdir) = parse_acf_installdir(&text) {
                let root = common.join(installdir);
                if data_file_path(&root).exists() {
                    return Some(root);
                }
            }
        }
    }

    // Known directory names observed in the working setup and common Steam names.
    let candidates = [
        "divinity2_dev_cut",
        "Divinity2_dev_cut",
        "Divinity II - Developer's Cut",
        "Divinity II Developers Cut",
        "Divinity II - Developers Cut",
        "Divinity 2 Developer's Cut",
        "Divinity 2 Developers Cut",
    ];
    for name in candidates {
        let root = common.join(name);
        if data_file_path(&root).exists() {
            return Some(root);
        }
    }

    None
}

fn parse_acf_installdir(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"installdir\"") {
            let quoted: Vec<&str> = trimmed.split('"').collect();
            if quoted.len() >= 4 {
                return Some(quoted[3].replace("\\\\", "\\"));
            }
        }
    }
    None
}

fn open_file(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("notepad.exe").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

fn open_folder(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

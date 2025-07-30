use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;
use rayon::prelude::*;
use blake3::Hasher;
use xxhash_rust::xxh3::xxh3_64;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use serde_json::Value;

// --- Data Structures for JSON Output ---

#[derive(Serialize)]
struct ReportData {
    dir1: String,
    dir2: String,
    total_files_checked: usize,
    total_size_checked: u64,
    total_duplicated_size: u64,
    duplicates: Vec<DuplicateSet>,
    size_by_ext: Value,
}

#[derive(Serialize)]
struct DuplicateSet {
    hash: String,
    size: u64,
    files: Vec<DuplicateFile>,
}

#[derive(Serialize)]
struct DuplicateFile {
    name: String,
    path: String,
    extension: String,
}


// --- Core Logic (largely unchanged) ---

const INITIAL_CHUNK_SIZE: usize = 4096;

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    size: u64,
}

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;

    if bytes >= TIB {
        format!("{:.2} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

fn hash_initial_chunk(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut buffer = [0; INITIAL_CHUNK_SIZE];
    let bytes_read = file.read(&mut buffer)?;
    Ok(xxh3_64(&buffer[..bytes_read]))
}

fn hash_full_file(path: &Path) -> io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buffer = [0; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_files(dir: &Path, pb: &ProgressBar) -> Vec<FileEntry> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_files(&path, pb));
            } else if let Ok(metadata) = entry.metadata() {
                if metadata.len() > 0 {
                    files.push(FileEntry {
                        path: path.clone(),
                        size: metadata.len(),
                    });
                    pb.inc(1);
                    pb.set_message(format!("Scanning: {}", path.display()));
                }
            }
        }
    }
    files
}

fn group_by_size(files: Vec<FileEntry>) -> HashMap<u64, Vec<PathBuf>> {
    let mut map = HashMap::new();
    for file in files {
        map.entry(file.size).or_insert_with(Vec::new).push(file.path);
    }
    map.into_iter().filter(|(_, paths)| paths.len() > 1).collect()
}

fn group_by_initial_hash(
    size_groups: HashMap<u64, Vec<PathBuf>>,
    pb: &ProgressBar,
) -> HashMap<u64, Vec<PathBuf>> {
    let total_groups = size_groups.len();
    pb.set_length(total_groups as u64);
    pb.set_position(0);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) - Hashing initial chunks")
        .unwrap()
        .progress_chars("#>-"));

    let initial_hash_groups: Vec<_> = size_groups
        .into_par_iter()
        .flat_map(|(_, paths)| {
            let mut inner_map = HashMap::new();
            for path in paths {
                if let Ok(hash) = hash_initial_chunk(&path) {
                    inner_map.entry(hash).or_insert_with(Vec::new).push(path);
                }
            }
            pb.inc(1);
            inner_map.into_iter().filter(|(_, v)| v.len() > 1).collect::<Vec<_>>()
        })
        .collect();
    
    let mut map = HashMap::new();
    for (hash, paths) in initial_hash_groups {
        map.entry(hash).or_insert_with(Vec::new).extend(paths);
    }
    map
}

fn find_duplicates(
    initial_hash_groups: HashMap<u64, Vec<PathBuf>>,
    pb: &ProgressBar,
) -> HashMap<String, Vec<PathBuf>> {
    let total_files_to_hash: u64 = initial_hash_groups.values().map(|v| v.len() as u64).sum();
    pb.set_length(total_files_to_hash);
    pb.set_position(0);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) - Performing full hash")
        .unwrap()
        .progress_chars("#>-"));

    let final_hashes: Vec<_> = initial_hash_groups
        .into_par_iter()
        .flat_map(|(_, paths)| {
            paths.into_par_iter().filter_map(|path| {
                pb.inc(1);
                pb.set_message(format!("Hashing: {}", path.display()));
                hash_full_file(&path).ok().map(|hash| (hash, path))
            }).collect::<Vec<_>>()
        })
        .collect();

    let mut duplicates = HashMap::new();
    for (hash, path) in final_hashes {
        duplicates.entry(hash).or_insert_with(Vec::new).push(path);
    }

    duplicates.into_iter().filter(|(_, paths)| paths.len() > 1).collect()
}

// --- New Reporting Logic ---

const HTML_TEMPLATE: &str = include_str!("report_template.html");

/// Writes the JSON data into the HTML report, then opens the report in a browser.
fn write_and_open_report(report_data: &ReportData) -> io::Result<()> {
    // Serialize the data to a JSON string.
    let json_data = serde_json::to_string(report_data)?;

    // Replace the placeholder in the HTML template with the actual JSON data.
    // This embeds the data directly into the HTML file.
    let report_html = HTML_TEMPLATE.replace("/*__JSON_DATA_PLACEHOLDER__*/{}", &json_data);

    let report_path = "duplicates_report.html";
    fs::write(report_path, report_html)?;

    // Open the HTML file in the default browser
    match opener::open(report_path) {
        Ok(_) => println!("Report opened in your default browser."),
        Err(e) => eprintln!("Could not open report in browser: {}", e),
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <directory1> <directory2>", args[0]);
        std::process::exit(1);
    }

    let dir1 = PathBuf::from(&args[1]);
    let dir2 = PathBuf::from(&args[2]);

    if !dir1.is_dir() || !dir2.is_dir() {
        eprintln!("Error: Both arguments must be valid directories.");
        std::process::exit(1);
    }

    let start_time = Instant::now();

    // Steps 1-4: Find duplicates (unchanged)
    println!("Step 1/4: Scanning for files...");
    let pb_scan = ProgressBar::new_spinner();
    pb_scan.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
    let mut all_files = collect_files(&dir1, &pb_scan);
    all_files.extend(collect_files(&dir2, &pb_scan));
    let total_files_checked = all_files.len();
    let total_size_checked: u64 = all_files.iter().map(|f| f.size).sum();
    pb_scan.finish_with_message(format!("Found {} files ({}).", total_files_checked, format_size(total_size_checked)));

    println!("Step 2/4: Grouping files by size...");
    let size_groups = group_by_size(all_files);
    println!("Found {} size groups with potential duplicates.", size_groups.len());

    println!("Step 3/4: Performing quick hash check...");
    let pb_initial_hash = ProgressBar::new(0);
    let initial_hash_groups = group_by_initial_hash(size_groups, &pb_initial_hash);
    pb_initial_hash.finish_with_message("Initial hash check complete.");
    println!("Found {} groups after initial hash check.", initial_hash_groups.len());

    println!("Step 4/4: Performing full hash on remaining files...");
    let pb_full_hash = ProgressBar::new(0);
    let duplicates_map = find_duplicates(initial_hash_groups, &pb_full_hash);
    pb_full_hash.finish_with_message("Full hash comparison complete.");

    // Step 5: Prepare data and generate interactive report
    let mut total_duplicated_size: u64 = 0;
    let mut duplicate_sets = Vec::new();
    let mut size_by_ext: HashMap<String, u64> = HashMap::new();

    for (hash, paths) in &duplicates_map {
        let size = paths.first().and_then(|p| fs::metadata(p).ok()).map_or(0, |m| m.len());
        total_duplicated_size += size * (paths.len() as u64 - 1);
        
        let files: Vec<DuplicateFile> = paths.iter().map(|p| {
            let extension = p.extension().and_then(OsStr::to_str).unwrap_or("other").to_lowercase();
            DuplicateFile {
                name: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                path: p.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned(),
                extension,
            }
        }).collect();

        // Aggregate size by extension for the chart
        if let Some(first_file) = files.first() {
            if paths.len() > 1 {
                let duplicated_size_for_set = size * (paths.len() as u64 -1);
                *size_by_ext.entry(first_file.extension.clone()).or_insert(0) += duplicated_size_for_set;
            }
        }

        duplicate_sets.push(DuplicateSet {
            hash: hash.clone(),
            size,
            files,
        });
    }

    let report_data = ReportData {
        dir1: dir1.display().to_string(),
        dir2: dir2.display().to_string(),
        total_files_checked,
        total_size_checked,
        total_duplicated_size,
        duplicates: duplicate_sets,
        size_by_ext: serde_json::to_value(&size_by_ext).unwrap(),
    };

    if report_data.duplicates.is_empty() {
        println!("\nNo duplicate files found.");
    } else {
        println!("\nFound {} sets of duplicates (totaling {}). Generating interactive report...", report_data.duplicates.len(), format_size(total_duplicated_size));
        if let Err(e) = write_and_open_report(&report_data) {
            eprintln!("Error writing or opening report: {}", e);
        } else {
            println!("Report 'duplicates_report.html' created successfully.");
        }
    }

    println!("\nTotal execution time: {:.2?}", start_time.elapsed());
}

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use rayon::prelude::*;
use blake3::Hasher;
use xxhash_rust::xxh3::xxh3_64;
use indicatif::{ProgressBar, ProgressStyle};

const INITIAL_CHUNK_SIZE: usize = 4096; // 4KB

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

/// Hashes a small initial chunk of the file for a quick check
fn hash_initial_chunk(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut buffer = [0; INITIAL_CHUNK_SIZE];
    let bytes_read = file.read(&mut buffer)?;
    Ok(xxh3_64(&buffer[..bytes_read]))
}

/// Computes the full BLAKE3 hash for a file
fn hash_full_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buffer = [0; 8192]; // 8KB buffer

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Recursively collects all files in a given directory
fn collect_files(dir: &Path, pb: &ProgressBar) -> Vec<FileEntry> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_files(&path, pb));
            } else if let Ok(metadata) = entry.metadata() {
                if metadata.len() > 0 { // Ignore empty files
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

/// Groups files by size, as only files with the same size can be duplicates
fn group_by_size(files: Vec<FileEntry>) -> HashMap<u64, Vec<PathBuf>> {
    let mut map = HashMap::new();
    for file in files {
        map.entry(file.size).or_insert_with(Vec::new).push(file.path);
    }
    // Keep only groups with more than one file
    map.into_iter().filter(|(_, paths)| paths.len() > 1).collect()
}

/// Groups files by the hash of their initial chunk
fn group_by_initial_hash(
    size_groups: HashMap<u64, Vec<PathBuf>>,
    pb: &ProgressBar,
) -> HashMap<u64, Vec<PathBuf>> {
    let mut map = HashMap::new();
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

    for (hash, paths) in initial_hash_groups {
        map.entry(hash).or_insert_with(Vec::new).extend(paths);
    }
    map
}


/// Finds duplicates by computing full file hashes for potential candidates
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

/// Writes the list of duplicate files to a Markdown file
fn write_report(
    duplicates: &HashMap<String, Vec<PathBuf>>,
    dir1: &Path,
    dir2: &Path,
    total_files_checked: usize,
    total_size_checked: u64,
    total_duplicated_size: u64,
) -> io::Result<()> {
    let mut file = File::create("duplicates.md")?;
    writeln!(file, "# Duplicate File Report")?;
    writeln!(file, "\nComparison between `{}` and `{}`.", dir1.display(), dir2.display())?;
    
    writeln!(file, "\n## Summary")?;
    writeln!(file, "- **Files Checked:** {}", total_files_checked)?;
    writeln!(file, "- **Total Size:** {}", format_size(total_size_checked))?;
    writeln!(file, "- **Duplicate Sets Found:** {}", duplicates.len())?;
    writeln!(file, "- **Total Duplicated Size:** {}", format_size(total_duplicated_size))?;

    if duplicates.is_empty() {
        return Ok(());
    }
    
    writeln!(file, "\n## Duplicate Sets")?;

    for (hash, paths) in duplicates {
        writeln!(file, "\n---")?;
        writeln!(file, "\n**Hash:** `{}`", hash)?;
        
        let file_size = if let Some(first_path) = paths.first() {
            fs::metadata(first_path).map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        
        writeln!(file, "\n| File Name | Path | Size |")?;
        writeln!(file, "|---|---|---|")?;

        for path in paths {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let parent_dir = path.parent().unwrap_or(Path::new("")).to_string_lossy();
            writeln!(file, "| `{}` | `{}` | {} |", file_name, parent_dir, format_size(file_size))?;
        }
        writeln!(file)?;
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

    // --- Step 1: Collect all files from both directories ---
    println!("Step 1/4: Scanning for files...");
    let pb_scan = ProgressBar::new_spinner();
    pb_scan.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
    let mut all_files = collect_files(&dir1, &pb_scan);
    all_files.extend(collect_files(&dir2, &pb_scan));
    let total_files_checked = all_files.len();
    let total_size_checked: u64 = all_files.iter().map(|f| f.size).sum();
    pb_scan.finish_with_message(format!("Found {} files ({}).", total_files_checked, format_size(total_size_checked)));

    // --- Step 2: Group files by size ---
    println!("Step 2/4: Grouping files by size...");
    let size_groups = group_by_size(all_files);
    println!("Found {} size groups with potential duplicates.", size_groups.len());

    // --- Step 3: Group by initial chunk hash ---
    println!("Step 3/4: Performing quick hash check on potential duplicates...");
    let pb_initial_hash = ProgressBar::new(0);
    let initial_hash_groups = group_by_initial_hash(size_groups, &pb_initial_hash);
    pb_initial_hash.finish_with_message("Initial hash check complete.");
    println!("Found {} groups after initial hash check.", initial_hash_groups.len());

    // --- Step 4: Full hash and find duplicates ---
    println!("Step 4/4: Performing full hash on remaining files...");
    let pb_full_hash = ProgressBar::new(0);
    let duplicates = find_duplicates(initial_hash_groups, &pb_full_hash);
    pb_full_hash.finish_with_message("Full hash comparison complete.");

    // --- Step 5: Calculate final stats and write report ---
    let mut total_duplicated_size: u64 = 0;
    for paths in duplicates.values() {
        if let Some(first_path) = paths.first() {
            if let Ok(metadata) = fs::metadata(first_path) {
                // Size of all copies minus one original
                total_duplicated_size += metadata.len() * (paths.len() as u64 - 1);
            }
        }
    }

    if duplicates.is_empty() {
        println!("\nNo duplicate files found.");
    } else {
        println!("\nFound {} sets of duplicates (totaling {}). Generating report...", duplicates.len(), format_size(total_duplicated_size));
        if let Err(e) = write_report(&duplicates, &dir1, &dir2, total_files_checked, total_size_checked, total_duplicated_size) {
            eprintln!("Error writing report: {}", e);
        } else {
            println!("Report 'duplicates.md' created successfully.");
        }
    }

    println!("\nTotal execution time: {:.2?}", start_time.elapsed());
}

![Rust Version](https://img.shields.io/badge/rust-1.88.0%2B-orange?logo=rust&style=flat-square)
[![GitHub release](https://img.shields.io/github/v/release/pip3-kill-me/duplicate_finder?include_prereleases&sort=semver&style=flat-square)](https://github.com/pip3-kill-me/duplicate_finder/releases)
![CLI Tool](https://img.shields.io/badge/interface-CLI-informational?logo=terminal&style=flat-square)
![GUI Output](https://img.shields.io/badge/output-GUI%20Report-blueviolet?logo=html5&style=flat-square)


# Duplicate File Finder

**Version:** 0.1.0\
A high-performance command-line tool written in Rust to find duplicate files between two directories. It generates a self-contained, interactive HTML report that allows you to sort, filter, and visualize the results.

---

## Features

- **High-Performance Scanning**\
  Built with Rust and powered by Rayon for parallel processing and the high-speed BLAKE3 hashing algorithm.

- **Efficient Duplicate Detection**\
  Uses a multi-step filtering process to minimize disk I/O and CPU usage:

  1. Groups files by **size**
  2. Compares a **small initial chunk** of same-sized files
  3. Performs a **full BLAKE3 hash** only on the remaining potential duplicates

- **Interactive HTML Report**\
  Generates a single, self-contained `duplicates_report.html` file with a rich user interface.

- **Data Visualization**\
  Includes a pie chart that shows the total size of duplicated files, broken down by file extension.

- **Sorting & Filtering**

  - Click any table header to sort the results
  - Use the dropdown menu to filter by file extension

- **Detailed Information**\
  Hover over any file name in the table to see its full content hash.

- **Project Information**\
  The report includes version and a direct link to the GitHub repo.

---
# Releases

### [v0.1.0 – Initial Release](https://github.com/pip3-kill-me/duplicate_finder/releases)

* Core duplicate detection pipeline (size → chunk → full hash)
* HTML report with filtering, sorting, and visualization
* Open report automatically on completion

---
## Prerequisites

Make sure you have the Rust toolchain installed.\
You can install it via [rustup](https://rustup.rs/).

---

## How to Compile

1. **Create a new project**

```bash
   cargo new duplicate_finder
   cd duplicate_finder
   ```

2. **Place the source files**

   * Copy your main Rust code into `src/main.rs`
   * Place the HTML template in `src/report_template.html`

3. **Add dependencies**
   Open the `Cargo.toml` file and add the following:

   ```toml
   [dependencies]
   rayon = "1.8"
   blake3 = "1.5"
   xxhash-rust = { version = "0.8", features = ["xxh3"] }
   indicatif = "0.17"
   opener = "0.7.1"
   serde = { version = "1.0", features = ["derive"] }
   serde_json = "1.0"
   ```

4. **Build for release**

   ```bash
   cargo build --release
   ```

   The compiled binary will be at:

   * `./target/release/duplicate_finder` (Linux/macOS)
   * `.\target\release\duplicate_finder.exe` (Windows)

---

## How to Use

Run the program from your terminal, providing two directories to compare:

### Windows

```cmd
.\target\release\duplicate_finder.exe "C:\Path\To\Directory1" "D:\Path\To\Directory2"
```

### Linux/macOS

```bash
./target/release/duplicate_finder "/path/to/directory1" "/path/to/directory2"
```

> 📝 Enclose paths in quotes if they contain spaces.
> 📊 The tool will generate and open a file named `duplicates_report.html`.

---

## Topics
```
rust
cli
file-deduplication
html-report
parallel-processing
rayon
blake3
xxhash
```

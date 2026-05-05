mod report;

use binate_core::{
    AbsolutePathNormalizer, BinaryProvider, BuildIdNormalizer, DiffConfig, LinkerVersionNormalizer,
    MmapBinaryProvider, Normalizer, NormalizerChain, SemanticDiff, TimestampNormalizer,
};
use clap::{Parser, Subcommand, ValueEnum};
use report::{JsonReporter, OutputFormat, ReportConfig, Reporter, SarifReporter, TerminalReporter};
use std::path::PathBuf;
use std::process;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "binate",
    version,
    about = "Reproducible build validator for Rust binaries",
    long_about = "Performs semantic diffs between two Rust binaries to identify sources of non-determinism."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose/trace output (set RUST_LOG for fine-grained control)
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Compare two binaries for reproducibility
    Compare {
        /// First binary (reference / "left" build)
        left: PathBuf,

        /// Second binary (candidate / "right" build)
        right: PathBuf,

        /// Output format
        #[arg(short, long, value_enum, default_value_t = OutputFmt::Terminal)]
        format: OutputFmt,

        /// Show disassembly diff for changed symbols (x86/x86-64 only)
        #[arg(long)]
        disasm: bool,

        /// Disable all built-in normalizers (raw byte diff)
        #[arg(long)]
        no_normalize: bool,

        /// Skip a specific normalizer by name (repeatable)
        #[arg(long = "skip-normalizer", value_name = "NAME")]
        skip_normalizers: Vec<String>,

        /// Ignore a section entirely (repeatable)
        #[arg(long = "ignore-section", value_name = "SECTION")]
        ignore_sections: Vec<String>,

        /// Context lines shown around changed instructions in terminal output
        #[arg(long, default_value_t = 3)]
        context: usize,

        /// Disable parallel section processing
        #[arg(long)]
        no_parallel: bool,

        /// Exit with code 1 when any differences are found (for CI)
        #[arg(long)]
        strict: bool,
    },

    /// List all sections and their sizes in a binary
    Inspect {
        binary: PathBuf,

        #[arg(short, long, value_enum, default_value_t = OutputFmt::Terminal)]
        format: OutputFmt,
    },

    /// List all symbols and their addresses in a binary
    Symbols {
        binary: PathBuf,

        /// Attempt to demangle symbol names
        #[arg(short, long)]
        demangle: bool,
    },
}

#[derive(Clone, ValueEnum)]
enum OutputFmt {
    Terminal,
    Json,
    Sarif,
}

fn main() {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .with_writer(std::io::stderr)
        .init();

    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    };

    process::exit(exit_code);
}

fn run(cli: Cli) -> binate_core::Result<i32> {
    match cli.command {
        Commands::Compare {
            left,
            right,
            format,
            disasm,
            no_normalize,
            skip_normalizers,
            ignore_sections,
            context,
            no_parallel,
            strict,
        } => {
            let left_bin = MmapBinaryProvider::open(&left)?;
            let right_bin = MmapBinaryProvider::open(&right)?;

            let normalizer = if no_normalize {
                NormalizerChain::new()
            } else if !skip_normalizers.is_empty() {
                let defaults: Vec<Box<dyn Normalizer>> = vec![
                    Box::new(BuildIdNormalizer),
                    Box::new(TimestampNormalizer),
                    Box::new(AbsolutePathNormalizer::new()),
                    Box::new(LinkerVersionNormalizer),
                ];
                let mut c = NormalizerChain::new();
                for n in defaults {
                    if !skip_normalizers.iter().any(|s| s == n.name()) {
                        c = c.push_boxed(n);
                    }
                }
                c
            } else {
                NormalizerChain::default()
            };

            let config = DiffConfig {
                normalizer,
                show_disasm: disasm,
                parallel: !no_parallel,
                ignored_sections: ignore_sections,
            };

            let result = SemanticDiff::compare(&left_bin, &right_bin, &config)?;

            let rep_cfg = ReportConfig {
                format: match format {
                    OutputFmt::Terminal => OutputFormat::Terminal,
                    OutputFmt::Json => OutputFormat::Json,
                    OutputFmt::Sarif => OutputFormat::Sarif,
                },
                context_lines: context,
                color: console::colors_enabled(),
            };

            let output: String = match format {
                OutputFmt::Terminal => TerminalReporter.render(&result, &rep_cfg)?,
                OutputFmt::Json => JsonReporter.render(&result, &rep_cfg)?,
                OutputFmt::Sarif => SarifReporter.render(&result, &rep_cfg)?,
            };

            print!("{output}");

            if strict && !result.identical {
                Ok(1)
            } else {
                Ok(0)
            }
        }

        Commands::Inspect { binary, format } => {
            let bin = MmapBinaryProvider::open(&binary)?;

            match format {
                OutputFmt::Json => {
                    let sections: Vec<_> = bin
                        .sections()
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "name": s.name,
                                "address": s.address,
                                "size": s.file_range.map(|(_, sz)| sz).unwrap_or(0),
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&sections)?);
                }
                _ => {
                    println!("{:<32} {:>18} {:>12}", "SECTION", "ADDRESS", "SIZE");
                    println!("{}", "-".repeat(64));
                    bin.visit_sections(&mut |name, _kind, addr, data| {
                        println!("{:<32} {:#018x} {:>12}", name, addr, data.len());
                        Ok(())
                    })?;
                }
            }
            Ok(0)
        }

        Commands::Symbols { binary, demangle: _ } => {
            let bin = MmapBinaryProvider::open(&binary)?;

            println!("{:<18} {:>12} {}", "ADDRESS", "SIZE", "NAME");
            println!("{}", "-".repeat(60));
            for sym in bin.symbol_table().iter() {
                println!("{:#018x} {:>12} {}", sym.address, sym.size, sym.name);
            }
            Ok(0)
        }
    }
}

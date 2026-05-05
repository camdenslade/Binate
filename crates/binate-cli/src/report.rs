use binate_core::{AnonymousDiff, DiffResult, Result, SymbolDiff};
use console::style;
use similar::{ChangeTag, TextDiff};
use std::fmt::Write as FmtWrite;

pub struct ReportConfig {
    pub context_lines: usize,
    pub color: bool,
}

pub trait Reporter {
    fn render(&self, result: &DiffResult, cfg: &ReportConfig) -> Result<String>;
}

pub struct TerminalReporter;
pub struct JsonReporter;
pub struct SarifReporter;

impl Reporter for TerminalReporter {
    fn render(&self, result: &DiffResult, cfg: &ReportConfig) -> Result<String> {
        let mut out = String::new();

        if result.identical {
            let msg = "Binaries are identical after normalization.";
            if cfg.color {
                writeln!(out, "{}", style(msg).green().bold()).ok();
            } else {
                writeln!(out, "{msg}").ok();
            }
            return Ok(out);
        }

        let header = format!(
            "Differences found between\n  LEFT:  {}\n  RIGHT: {}",
            result.left_path.display(),
            result.right_path.display()
        );
        if cfg.color {
            writeln!(out, "{}", style(header).bold()).ok();
        } else {
            writeln!(out, "{header}").ok();
        }
        writeln!(out).ok();

        if !result.sections_only_in_left.is_empty() {
            writeln!(out, "Sections only in LEFT:").ok();
            for s in &result.sections_only_in_left {
                writeln!(out, "  {s}").ok();
            }
        }
        if !result.sections_only_in_right.is_empty() {
            writeln!(out, "Sections only in RIGHT:").ok();
            for s in &result.sections_only_in_right {
                writeln!(out, "  {s}").ok();
            }
        }

        for sd in &result.symbol_diffs {
            render_symbol_diff(&mut out, sd, cfg);
        }

        for ad in &result.anonymous_diffs {
            render_anon_diff(&mut out, ad, cfg);
        }

        Ok(out)
    }
}

fn render_symbol_diff(out: &mut String, sd: &SymbolDiff, cfg: &ReportConfig) {
    let loc = sd
        .source_location
        .as_ref()
        .map(|l| {
            format!(
                " ({}:{})",
                l.file,
                l.line.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
            )
        })
        .unwrap_or_default();

    let heading = format!("▸ symbol: {}{loc}", sd.symbol.name);
    if cfg.color {
        writeln!(out, "{}", style(&heading).yellow().bold()).ok();
    } else {
        writeln!(out, "{heading}").ok();
    }

    writeln!(out, "  address: {:#010x}  size: {} bytes", sd.symbol.address, sd.symbol.size).ok();
    writeln!(out, "  changed ranges: {}", sd.ranges.len()).ok();

    for r in &sd.ranges {
        writeln!(
            out,
            "  [{:#010x}+{:3}]  left={:?}  right={:?}",
            r.offset,
            r.len,
            &r.left_bytes[..r.left_bytes.len().min(16)],
            &r.right_bytes[..r.right_bytes.len().min(16)]
        )
        .ok();
    }

    // Disassembly diff (if available)
    if let (Some(dl), Some(dr)) = (&sd.disasm_left, &sd.disasm_right) {
        let left_text = dl.join("\n");
        let right_text = dr.join("\n");
        let diff = TextDiff::from_lines(&left_text, &right_text);
        writeln!(out, "  --- disasm diff ---").ok();
        for group in diff.grouped_ops(cfg.context_lines) {
            for op in &group {
                for change in diff.iter_changes(op) {
                    let tag = change.tag();
                    let line = change.value();
                    let sign = match tag {
                        ChangeTag::Delete => "-",
                        ChangeTag::Insert => "+",
                        ChangeTag::Equal  => " ",
                    };
                    if cfg.color {
                        let styled = match tag {
                            ChangeTag::Delete => style(format!("  {sign} {line}")).red().to_string(),
                            ChangeTag::Insert => style(format!("  {sign} {line}")).green().to_string(),
                            ChangeTag::Equal  => format!("    {line}"),
                        };
                        write!(out, "{styled}").ok();
                    } else {
                        write!(out, "  {sign} {line}").ok();
                    }
                }
            }
        }
    }

    writeln!(out).ok();
}

fn render_anon_diff(out: &mut String, ad: &AnonymousDiff, cfg: &ReportConfig) {
    let heading = format!("▸ section: {} (unattributed)", ad.section);
    if cfg.color {
        writeln!(out, "{}", style(&heading).cyan().bold()).ok();
    } else {
        writeln!(out, "{heading}").ok();
    }
    writeln!(out, "  changed ranges: {}", ad.ranges.len()).ok();
    for r in &ad.ranges {
        writeln!(
            out,
            "  [{:#010x}+{:3}]  left={:?}  right={:?}",
            r.offset,
            r.len,
            &r.left_bytes[..r.left_bytes.len().min(16)],
            &r.right_bytes[..r.right_bytes.len().min(16)]
        )
        .ok();
    }
    writeln!(out).ok();
}

impl Reporter for JsonReporter {
    fn render(&self, result: &DiffResult, _cfg: &ReportConfig) -> Result<String> {
        Ok(serde_json::to_string_pretty(result)?)
    }
}

impl Reporter for SarifReporter {
    fn render(&self, result: &DiffResult, _cfg: &ReportConfig) -> Result<String> {
        let mut results = Vec::new();

        for sd in &result.symbol_diffs {
            let (file, line) = sd
                .source_location
                .as_ref()
                .map(|l| (l.file.clone(), l.line.unwrap_or(1)))
                .unwrap_or_else(|| (String::from("<unknown>"), 1));

            results.push(serde_json::json!({
                "ruleId": "BINATE001",
                "level": "warning",
                "message": {
                    "text": format!(
                        "Non-deterministic output in symbol '{}': {} byte range(s) differ",
                        sd.symbol.name,
                        sd.ranges.len()
                    )
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": file },
                        "region": { "startLine": line }
                    }
                }]
            }));
        }

        for ad in &result.anonymous_diffs {
            results.push(serde_json::json!({
                "ruleId": "BINATE002",
                "level": "warning",
                "message": {
                    "text": format!(
                        "Non-deterministic output in section '{}': {} byte range(s) differ",
                        ad.section,
                        ad.ranges.len()
                    )
                },
                "locations": []
            }));
        }

        let sarif = serde_json::json!({
            "$schema": "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json",
            "version": "2.1.0",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "binate",
                        "version": env!("CARGO_PKG_VERSION"),
                        "rules": [
                            {
                                "id": "BINATE001",
                                "name": "SymbolNonDeterminism",
                                "shortDescription": { "text": "Symbol bytes differ between builds" }
                            },
                            {
                                "id": "BINATE002",
                                "name": "SectionNonDeterminism",
                                "shortDescription": { "text": "Unattributed section bytes differ between builds" }
                            }
                        ]
                    }
                },
                "artifacts": [
                    { "location": { "uri": result.left_path.to_string_lossy() } },
                    { "location": { "uri": result.right_path.to_string_lossy() } }
                ],
                "results": results
            }]
        });

        Ok(serde_json::to_string_pretty(&sarif)?)
    }
}

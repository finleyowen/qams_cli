use std::{env, fs, path::{Path, PathBuf}, process::Command};
use qams_core::Scorecard;

/// Runs `init` (and `update`, which is identical) in the current directory.
pub fn run(
    path_to_scorecard: &Path,
    path_to_agents:    &Path,
    path_to_metadata:  Option<&Path>,
) -> Result<(), String> {
    let target = env::current_dir()
        .map_err(|e| format!("Cannot determine current directory: {e}"))?;

    // ── 1. Parse inputs ───────────────────────────────────────────────────

    let scorecard_csv = fs::read_to_string(path_to_scorecard)
        .map_err(|e| format!("Cannot read '{}': {e}", path_to_scorecard.display()))?;
    let scorecard = Scorecard::from_csv_string(&scorecard_csv)?;

    let agents_csv = fs::read_to_string(path_to_agents)
        .map_err(|e| format!("Cannot read '{}': {e}", path_to_agents.display()))?;
    let agent_rows = parse_csv_rows(&agents_csv);
    if agent_rows.is_empty() {
        return Err("Agents CSV contains no rows".into());
    }
    for (i, row) in agent_rows.iter().enumerate() {
        if row.is_empty() || row[0].is_empty() {
            return Err(format!("Agent row {} has an empty identifier", i + 1));
        }
    }

    // Optional metadata: one field name per row (first column).
    let metadata_fields: Vec<String> = match path_to_metadata {
        None => Vec::new(),
        Some(p) => {
            let csv = fs::read_to_string(p)
                .map_err(|e| format!("Cannot read '{}': {e}", p.display()))?;
            parse_csv_rows(&csv)
                .into_iter()
                .filter_map(|mut row| row.drain(..).next())
                .filter(|f| !f.is_empty())
                .collect()
        }
    };

    // ── 2. Create directory structure ─────────────────────────────────────

    mkdir(&target.join("reviews"))?;
    mkdir(&target.join("reports"))?;
    let qams_dir = target.join(".qams");
    mkdir(&qams_dir)?;

    // ── 3. Write scorecard.html ───────────────────────────────────────────

    let agent_row_refs: Vec<Vec<&str>> = agent_rows
        .iter()
        .map(|row| row.iter().map(|s| s.as_str()).collect())
        .collect();
    let agent_slices: Vec<&[&str]> = agent_row_refs.iter().map(|r| r.as_slice()).collect();

    let mut html = scorecard.to_html(&agent_slices);

    // If metadata fields were provided, inject extra text inputs into the
    // meta-grid after the date field.
    if !metadata_fields.is_empty() {
        let extra_inputs: String = metadata_fields
            .iter()
            .map(|field| {
                let id  = sanitise_id(field);
                let esc = escape_html(field);
                format!(
                    "<div class=\"field\">\n          \
                     <label for=\"{id}\">{esc}</label>\n          \
                     <input type=\"text\" id=\"{id}\" name=\"{id}\" placeholder=\"{esc}\">\n        \
                     </div>"
                )
            })
            .collect::<Vec<_>>()
            .join("\n        ");

        // Append after the date field's closing </div>, inside .meta-grid.
        html = html.replacen(
            "<input type=\"date\" id=\"date\" name=\"date\">\n        </div>\n      </div>",
            &format!(
                "<input type=\"date\" id=\"date\" name=\"date\">\n        </div>\n        {extra_inputs}\n      </div>"
            ),
            1,
        );
    }

    write_file(&target.join("scorecard.html"), &html)?;

    // ── 4. Hide .qams ─────────────────────────────────────────────────────

    if cfg!(target_os = "windows") {
        if let Err(e) = Command::new("attrib").arg("+H").arg(&qams_dir).status() {
            eprintln!("Warning: could not hide .qams: {e}");
        }
    }

    // ── 5. Report success ─────────────────────────────────────────────────

    println!("QAMS instance ready in {}", target.display());
    println!("  reviews/        — place completed review JSON files here");
    println!("  reports/        — generated reports will appear here");
    println!("  scorecard.html  — open this to conduct a review");

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parses a CSV string into rows of trimmed cell strings, skipping blank lines.
fn parse_csv_rows(csv: &str) -> Vec<Vec<String>> {
    csv.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').map(|c| c.trim().to_string()).collect())
        .collect()
}

fn mkdir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|e| format!("Cannot create '{}': {e}", path.display()))
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content)
        .map_err(|e| format!("Cannot write '{}': {e}", path.display()))
}

fn escape_html(s: &str) -> String {
    s.chars().map(|c| match c {
        '&'  => "&amp;".to_string(),
        '<'  => "&lt;".to_string(),
        '>'  => "&gt;".to_string(),
        '"'  => "&quot;".to_string(),
        '\'' => "&#39;".to_string(),
        c    => c.to_string(),
    }).collect()
}

fn sanitise_id(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Resolved relative to qams_cli/src/init.rs → ../../qams_core/test_artifacts/
    const SCORECARD_CSV: &str =
        include_str!("../../qams_core/test_artifacts/vsc1.csv");

    const AGENTS_CSV: &str = "Alice Nguyen,Team A\nBen Carter,Team B\nClara Singh,Team A\n";
    const METADATA_CSV: &str = "Call ID\nChannel\n";

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "qams_cli_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    /// Temporarily changes cwd to `tmp`, runs `run(...)`, then restores cwd.
    fn run_in(tmp: &Path, sc: &Path, ag: &Path, meta: Option<&Path>) -> Result<(), String> {
        let original = env::current_dir().unwrap();
        env::set_current_dir(tmp).unwrap();
        let result = run(sc, ag, meta);
        env::set_current_dir(original).unwrap();
        result
    }

    #[test]
    fn creates_directory_structure() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_in(&tmp, &sc, &ag, None).unwrap();

        assert!(tmp.join("reviews").is_dir(),       "reviews/ missing");
        assert!(tmp.join("reports").is_dir(),       "reports/ missing");
        assert!(tmp.join(".qams").is_dir(),         ".qams/ missing");
        assert!(tmp.join("scorecard.html").is_file(), "scorecard.html missing");
    }

    #[test]
    fn scorecard_html_contains_agents() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_in(&tmp, &sc, &ag, None).unwrap();

        let html = fs::read_to_string(tmp.join("scorecard.html")).unwrap();
        assert!(html.contains("Alice Nguyen"), "Alice Nguyen missing");
        assert!(html.contains("Ben Carter"),   "Ben Carter missing");
    }

    #[test]
    fn metadata_fields_injected() {
        let tmp  = tempdir();
        let sc   = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag   = write(&tmp, "agents.csv", AGENTS_CSV);
        let meta = write(&tmp, "meta.csv",   METADATA_CSV);
        run_in(&tmp, &sc, &ag, Some(&meta)).unwrap();

        let html = fs::read_to_string(tmp.join("scorecard.html")).unwrap();
        assert!(html.contains("call_id"), "'call_id' input missing");
        assert!(html.contains("Channel"), "'Channel' label missing");
    }

    #[test]
    fn empty_agents_is_error() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", "");
        assert!(run_in(&tmp, &sc, &ag, None).is_err());
    }

    #[test]
    fn idempotent() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_in(&tmp, &sc, &ag, None).unwrap();
        assert!(run_in(&tmp, &sc, &ag, None).is_ok(), "second run should succeed");
    }
}
use std::{env, fs, path::{Path, PathBuf}, process::Command};
use qams_core::Scorecard;

const DEFAULT_PATH_TO_REVIEWS: &str = "./reviews";
const DEFAULT_PATH_TO_REPORTS: &str = "./reports";
const DEFAULT_ACCUMULATION_PERIOD: u32 = 4;

/// Runs `init` in the current directory. `path_to_scorecard` and
/// `path_to_agents` are required (callers should apply CLI defaults before
/// calling this, e.g. `./scorecard.csv` / `./agents.csv`).
pub fn run_init(
    path_to_scorecard: &Path,
    path_to_agents:    &Path,
    path_to_metadata:  Option<&Path>,
) -> Result<(), String> {
    let target = current_dir()?;

    let scorecard = read_scorecard(path_to_scorecard)?;
    let agent_rows = read_agents(path_to_agents)?;
    let metadata_fields = read_metadata(path_to_metadata)?;

    mkdir(&target.join("reviews"))?;
    mkdir(&target.join("reports"))?;
    let qams_dir = target.join(".qams");
    mkdir(&qams_dir)?;
    hide_dir(&qams_dir);

    write_scorecard_html(&target, &scorecard, &agent_rows, &metadata_fields)?;

    // options.toml is user-visible and user-editable, so it lives at the
    // instance root rather than inside the hidden .qams directory.
    let options_path = target.join("options.toml");
    if !options_path.exists() {
        write_file(&options_path, &default_options_toml())?;
    }

    println!("QAMS instance ready in {}", target.display());
    println!("  reviews/        — place completed review JSON files here");
    println!("  reports/        — generated reports will appear here");
    println!("  scorecard.html  — open this to conduct a review");
    println!("  options.toml    — instance settings (safe to edit)");

    Ok(())
}

/// Runs `update` in the current directory. Any argument left as `None`
/// leaves the corresponding part of the existing QAMS instance unchanged.
/// If none of `path_to_scorecard`/`path_to_agents`/`path_to_metadata` were
/// given, `scorecard.html` is left untouched entirely.
pub fn run_update(
    path_to_scorecard: Option<&Path>,
    path_to_agents:    Option<&Path>,
    path_to_metadata:  Option<&Path>,
) -> Result<(), String> {
    let target = current_dir()?;

    if path_to_scorecard.is_none() && path_to_agents.is_none() && path_to_metadata.is_none() {
        println!("Nothing to update — no paths were provided.");
        return Ok(());
    }

    // For any omitted input, fall back to re-deriving it from the existing
    // scorecard.html where possible isn't feasible (HTML isn't a source of
    // truth), so an omitted scorecard/agents path means: regenerate
    // scorecard.html is skipped for that piece only if *both* are omitted.
    // Since to_html requires both a Scorecard and an agent list together,
    // updating just one of the two still requires the other's most recent
    // *input* CSV. We therefore require scorecard and agents to be updated
    // together if either changes; metadata can be updated independently of
    // both by being merged into a freshly-generated scorecard.html.
    let scorecard_csv_path = path_to_scorecard;
    let agents_csv_path    = path_to_agents;

    if (scorecard_csv_path.is_some()) != (agents_csv_path.is_some()) {
        return Err(
            "Updating the scorecard or the agent list requires both \
             -s/--path-to-scorecard and -a/--path-to-agents to be provided \
             together, since scorecard.html is regenerated from both."
                .into(),
        );
    }

    if scorecard_csv_path.is_some() {
        let scorecard = read_scorecard(scorecard_csv_path.unwrap())?;
        let agent_rows = read_agents(agents_csv_path.unwrap())?;
        let metadata_fields = read_metadata(path_to_metadata)?;
        write_scorecard_html(&target, &scorecard, &agent_rows, &metadata_fields)?;
        println!("Updated scorecard.html (scorecard, agents{}).",
            if path_to_metadata.is_some() { ", metadata" } else { "" });
    } else if let Some(meta_path) = path_to_metadata {
        // Metadata-only update: scorecard/agents are unchanged, but we still
        // need both source CSVs to regenerate scorecard.html. Since we don't
        // persist them, ask the user to supply scorecard+agents alongside
        // metadata, or accept that metadata-only updates aren't supported
        // without re-supplying scorecard+agents.
        let _ = meta_path;
        return Err(
            "Updating review metadata also requires -s/--path-to-scorecard \
             and -a/--path-to-agents, since scorecard.html must be \
             regenerated from all three inputs together."
                .into(),
        );
    }

    Ok(())
}

// ── Shared steps ────────────────────────────────────────────────────────────

fn current_dir() -> Result<PathBuf, String> {
    env::current_dir().map_err(|e| format!("Cannot determine current directory: {e}"))
}

fn read_scorecard(path: &Path) -> Result<Scorecard, String> {
    let csv = fs::read_to_string(path)
        .map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;
    Scorecard::from_csv_string(&csv)
}

fn read_agents(path: &Path) -> Result<Vec<Vec<String>>, String> {
    let csv = fs::read_to_string(path)
        .map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;
    let rows = parse_csv_rows(&csv);
    if rows.is_empty() {
        return Err("Agents CSV contains no rows".into());
    }
    for (i, row) in rows.iter().enumerate() {
        if row.is_empty() || row[0].is_empty() {
            return Err(format!("Agent row {} has an empty identifier", i + 1));
        }
    }
    Ok(rows)
}

fn read_metadata(path: Option<&Path>) -> Result<Vec<String>, String> {
    match path {
        None => Ok(Vec::new()),
        Some(p) => {
            let csv = fs::read_to_string(p)
                .map_err(|e| format!("Cannot read '{}': {e}", p.display()))?;
            Ok(parse_csv_rows(&csv)
                .into_iter()
                .filter_map(|mut row| row.drain(..).next())
                .filter(|f| !f.is_empty())
                .collect())
        }
    }
}

fn write_scorecard_html(
    target: &Path,
    scorecard: &Scorecard,
    agent_rows: &[Vec<String>],
    metadata_fields: &[String],
) -> Result<(), String> {
    let agent_row_refs: Vec<Vec<&str>> = agent_rows
        .iter()
        .map(|row| row.iter().map(|s| s.as_str()).collect())
        .collect();
    let agent_slices: Vec<&[&str]> = agent_row_refs.iter().map(|r| r.as_slice()).collect();

    let mut html = scorecard.to_html(&agent_slices);

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

        html = html.replacen(
            "<input type=\"date\" id=\"date\" name=\"date\">\n        </div>\n      </div>",
            &format!(
                "<input type=\"date\" id=\"date\" name=\"date\">\n        </div>\n        {extra_inputs}\n      </div>"
            ),
            1,
        );
    }

    write_file(&target.join("scorecard.html"), &html)
}

fn default_options_toml() -> String {
    format!(
        "path_to_reviews = \"{}\"\npath_to_reports = \"{}\"\naccumulation_period = {}\n",
        DEFAULT_PATH_TO_REVIEWS,
        DEFAULT_PATH_TO_REPORTS,
        DEFAULT_ACCUMULATION_PERIOD,
    )
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

fn hide_dir(path: &Path) {
    if cfg!(target_os = "windows") {
        if let Err(e) = Command::new("attrib").arg("+H").arg(path).status() {
            eprintln!("Warning: could not hide '{}': {e}", path.display());
        }
    }
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

    fn run_init_in(tmp: &Path, sc: &Path, ag: &Path, meta: Option<&Path>) -> Result<(), String> {
        let original = env::current_dir().unwrap();
        env::set_current_dir(tmp).unwrap();
        let result = run_init(sc, ag, meta);
        env::set_current_dir(original).unwrap();
        result
    }

    fn run_update_in(
        tmp: &Path,
        sc: Option<&Path>,
        ag: Option<&Path>,
        meta: Option<&Path>,
    ) -> Result<(), String> {
        let original = env::current_dir().unwrap();
        env::set_current_dir(tmp).unwrap();
        let result = run_update(sc, ag, meta);
        env::set_current_dir(original).unwrap();
        result
    }

    #[test]
    fn creates_directory_structure() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();

        assert!(tmp.join("reviews").is_dir(),         "reviews/ missing");
        assert!(tmp.join("reports").is_dir(),         "reports/ missing");
        assert!(tmp.join(".qams").is_dir(),           ".qams/ missing");
        assert!(tmp.join("scorecard.html").is_file(), "scorecard.html missing");
        assert!(tmp.join("options.toml").is_file(),   "options.toml missing");
    }

    #[test]
    fn options_toml_has_expected_defaults() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();

        let toml = fs::read_to_string(tmp.join("options.toml")).unwrap();
        assert!(toml.contains("path_to_reviews = \"./reviews\""));
        assert!(toml.contains("path_to_reports = \"./reports\""));
        assert!(toml.contains("accumulation_period = 4"));
    }

    #[test]
    fn options_toml_not_overwritten_on_reinit() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();

        // Simulate the user customising their options.
        fs::write(tmp.join("options.toml"), "custom = true\n").unwrap();

        run_init_in(&tmp, &sc, &ag, None).unwrap();
        let toml = fs::read_to_string(tmp.join("options.toml")).unwrap();
        assert_eq!(toml, "custom = true\n", "existing options.toml should not be clobbered");
    }

    #[test]
    fn scorecard_html_contains_agents() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();

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
        run_init_in(&tmp, &sc, &ag, Some(&meta)).unwrap();

        let html = fs::read_to_string(tmp.join("scorecard.html")).unwrap();
        assert!(html.contains("call_id"), "'call_id' input missing");
        assert!(html.contains("Channel"), "'Channel' label missing");
    }

    #[test]
    fn empty_agents_is_error() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", "");
        assert!(run_init_in(&tmp, &sc, &ag, None).is_err());
    }

    #[test]
    fn idempotent() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();
        assert!(run_init_in(&tmp, &sc, &ag, None).is_ok(), "second init should succeed");
    }

    #[test]
    fn update_with_no_args_is_a_noop() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();
        assert!(run_update_in(&tmp, None, None, None).is_ok());
    }

    #[test]
    fn update_scorecard_and_agents_together() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();

        let ag2 = write(&tmp, "agents2.csv", "Dana Lee\n");
        run_update_in(&tmp, Some(&sc), Some(&ag2), None).unwrap();

        let html = fs::read_to_string(tmp.join("scorecard.html")).unwrap();
        assert!(html.contains("Dana Lee"), "updated agent missing from scorecard.html");
    }

    #[test]
    fn update_scorecard_without_agents_is_error() {
        let tmp = tempdir();
        let sc  = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag  = write(&tmp, "agents.csv", AGENTS_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();
        assert!(run_update_in(&tmp, Some(&sc), None, None).is_err());
    }

    #[test]
    fn update_metadata_without_scorecard_and_agents_is_error() {
        let tmp  = tempdir();
        let sc   = write(&tmp, "sc.csv",     SCORECARD_CSV);
        let ag   = write(&tmp, "agents.csv", AGENTS_CSV);
        let meta = write(&tmp, "meta.csv",   METADATA_CSV);
        run_init_in(&tmp, &sc, &ag, None).unwrap();
        assert!(run_update_in(&tmp, None, None, Some(&meta)).is_err());
    }
}
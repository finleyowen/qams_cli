//! CSV parsing and HTML rendering for `Scorecard`.
//!
//! These used to live in `qams_core::Scorecard` directly, but since they're
//! presentation/IO concerns specific to this CLI rather than core scoring
//! logic, they've been moved here. `qams_core::Scorecard` exposes a `new`
//! constructor plus read-only accessors (`criteria`, `criterion`,
//! `criterion_order`, `option_order`) that this module relies on.

use std::collections::{HashMap, HashSet};
use qams_core::{Criterion, CriterionScore, Scorecard};

/// Parses a scorecard from a CSV string.
///
/// Format:
/// - Header row: empty first cell, then option names (must be unique)
/// - Each subsequent row: criterion name in first cell, then per-option values
///   - A number   → `Points(n)`
///   - `"N"`      → `NotApplicable`
///   - `"F"`      → `Autofail`
///   - empty      → option not available on this criterion (omitted)
pub fn parse_scorecard_csv(csv: &str) -> Result<Scorecard, String> {
    let mut lines = csv.lines();

    // --- header row ---
    let header_line = lines.next().ok_or("CSV is empty")?;
    let header_cells: Vec<&str> = header_line.split(',').collect();
    // first cell is the top-left corner (ignored)
    let option_names: Vec<&str> = header_cells[1..].to_vec();

    // Enforce unique option names
    {
        let mut seen = HashSet::new();
        for name in &option_names {
            if !seen.insert(*name) {
                return Err(format!("Duplicate option name: '{name}'"));
            }
        }
    }

    // --- criterion rows ---
    let mut criteria: HashMap<String, Criterion> = HashMap::new();
    let mut criterion_order: Vec<String> = Vec::new();

    for (row_idx, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let crit_name = cells[0];
        if crit_name.is_empty() {
            return Err(format!("Row {} has an empty criterion name", row_idx + 2));
        }
        if criteria.contains_key(crit_name) {
            return Err(format!("Duplicate criterion name: '{crit_name}'"));
        }

        let mut options: HashMap<String, CriterionScore> = HashMap::new();

        for (col_idx, opt_name) in option_names.iter().enumerate() {
            let cell = cells.get(col_idx + 1).copied().unwrap_or("").trim();
            let score = match cell {
                "" => continue, // option not available on this criterion
                "N" => CriterionScore::NotApplicable,
                "F" => CriterionScore::Autofail,
                other => {
                    let points: u32 = other.parse().map_err(|_| {
                        format!(
                            "Invalid cell value '{}' at criterion '{}', option '{}'",
                            other, crit_name, opt_name
                        )
                    })?;
                    CriterionScore::Points(points)
                }
            };
            options.insert(opt_name.to_string(), score);
        }

        criterion_order.push(crit_name.to_string());
        criteria.insert(crit_name.to_string(), Criterion::new(options));
    }

    Ok(Scorecard::new(
        criteria,
        option_names.iter().map(|s| s.to_string()).collect(),
        criterion_order,
    ))
}

/// Renders the scorecard as a self-contained HTML review form.
///
/// The returned string is a full HTML document that a QA officer fills in
/// for each review. Submitting the form serialises the selections and
/// optional per-criterion comments to JSON, which the CLI writes to the
/// `reviews/` directory.
///
/// `agents` mirrors the agents CSV: each inner slice is one row, where
/// `agents[n][0]` is the agent's unique identifier (used as the option
/// value) and any further elements are metadata columns, stored as
/// `data-meta-N` attributes on the `<option>` element for future use.
///
/// Option buttons are styled to reflect their score type:
/// - **Green**  – the highest-point value on that criterion (full marks)
/// - **Blue**   – a Points value below the maximum
/// - **Grey**   – NotApplicable (`N` in the CSV)
/// - **Red**    – Autofail (`F` in the CSV)
/// - **Dash**   – option not available on this criterion (no button rendered)
pub fn render_scorecard_html(scorecard: &Scorecard, agents: &[&[&str]]) -> String {
    let option_headers: String = scorecard.option_order()
        .iter()
        .map(|o| format!("<th>{}</th>", escape_html(o)))
        .collect();

    let agent_options: String = agents
        .iter()
        .filter(|row| !row.is_empty())
        .map(|row| {
            let id = escape_html(row[0]);
            let meta_attrs: String = row[1..]
                .iter()
                .enumerate()
                .map(|(i, v)| format!(" data-meta-{}=\"{}\"", i, escape_html(v)))
                .collect();
            format!("<option value=\"{id}\"{meta_attrs}>{id}</option>")
        })
        .collect::<Vec<_>>()
        .join("\n            ");

    let criterion_rows: String = scorecard.criterion_order()
        .iter()
        .map(|crit_name| {
            let criterion = scorecard.criterion(crit_name)
                .expect("criterion_order must be consistent with criteria");

            // Highest Points value on this criterion — used to identify
            // the "full marks" option for green styling.
            let max_pts = criterion.max_points();

            let option_cells: String = scorecard.option_order()
                .iter()
                .map(|opt_name| {
                    match criterion.option(opt_name) {
                        None => {
                            // Empty cell in the CSV — option not available.
                            "<td class=\"opt-cell opt-unavailable\"><span aria-hidden=\"true\">—</span></td>".to_string()
                        }
                        Some(score) => {
                            let (css_class, title) = match score {
                                CriterionScore::Autofail =>
                                    ("opt-autofail", "Autofail"),
                                CriterionScore::NotApplicable =>
                                    ("opt-na", "Not applicable"),
                                CriterionScore::Points(p) if *p == max_pts && max_pts > 0 =>
                                    ("opt-full", "Full points"),
                                CriterionScore::Points(_) =>
                                    ("opt-points", "Partial points"),
                            };
                            let id = format!(
                                "{}_{}",
                                escape_html(crit_name),
                                escape_html(opt_name)
                            );
                            format!(
                                r#"<td class="opt-cell">
  <label class="opt-label {css_class}" title="{title}">
    <input type="radio" name="{crit}" value="{opt}" id="{id}" onchange="recalc()">
    <span class="opt-btn">{opt_display}</span>
  </label>
</td>"#,
                                crit = escape_html(crit_name),
                                opt  = escape_html(opt_name),
                                opt_display = escape_html(opt_name),
                            )
                        }
                    }
                })
                .collect();

            let uid = crit_name.replace(|c: char| !c.is_alphanumeric(), "_");
            format!(
                r#"<tr>
  <td class="crit-name"><code>{crit}</code></td>
  {option_cells}
  <td class="comment-cell">
    <button type="button" class="comment-toggle"
            onclick="toggleComment('{uid}')"
            aria-expanded="false" aria-controls="comment-{uid}">
      + comment
    </button>
    <div class="comment-area" id="comment-{uid}" hidden>
      <textarea name="comment_{uid}"
                aria-label="Comment for {crit}"
                placeholder="Optional comment…"></textarea>
    </div>
  </td>
</tr>"#,
                crit = escape_html(crit_name),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>QAMS – Review Form</title>
<style>
*, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
:root {{
  --ink:        #1a1a1a;
  --ink-mid:    #555;
  --ink-soft:   #888;
  --rule:       #e0ddd8;
  --surface:    #f9f8f6;
  --white:      #ffffff;
  --accent:     #2563eb;
  --accent-bg:  #eff4ff;
  --danger:     #c0392b;
  --danger-bg:  #fdf2f2;
  --success:    #166534;
  --success-bg: #f0fdf4;
  --na-color:   #4b5563;
  --na-bg:      #f3f4f6;
  --radius:     6px;
  --font:       "Inter", "Segoe UI", system-ui, sans-serif;
  --mono:       "JetBrains Mono", "Fira Code", "Consolas", monospace;
}}
html {{ font-size: 15px; }}
body {{ font-family: var(--font); color: var(--ink); background: var(--surface); min-height: 100vh; padding-bottom: 80px; }}

/* header */
.page-header {{ background: var(--white); border-bottom: 1px solid var(--rule); padding: 18px 40px; display: flex; align-items: center; gap: 12px; }}
.wordmark {{ font-size: 12px; font-weight: 700; letter-spacing: 0.14em; text-transform: uppercase; color: var(--ink-soft); }}
.sep {{ color: var(--rule); }}
.page-title {{ font-size: 15px; font-weight: 600; }}

/* layout */
.shell {{ max-width: 900px; margin: 36px auto 0; padding: 0 24px; }}
.section-label {{ font-size: 11px; font-weight: 600; letter-spacing: 0.1em; text-transform: uppercase; color: var(--ink-soft); margin-bottom: 10px; }}
.card {{ background: var(--white); border: 1px solid var(--rule); border-radius: var(--radius); margin-bottom: 20px; }}
.card-body {{ padding: 22px; }}

/* meta fields */
.meta-grid {{ display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 18px; }}
@media (max-width: 600px) {{ .meta-grid {{ grid-template-columns: 1fr; }} }}
.field label {{ display: block; font-size: 12px; font-weight: 600; color: var(--ink-mid); margin-bottom: 5px; }}
.field input, .field select {{
  width: 100%; padding: 8px 10px; font-family: var(--font); font-size: 14px;
  color: var(--ink); background: var(--white); border: 1px solid var(--rule);
  border-radius: var(--radius); appearance: none; outline: none; transition: border-color .15s;
}}
.field input:focus, .field select:focus {{ border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-bg); }}

/* score */
.score-bar-card {{ display: flex; align-items: center; gap: 20px; background: var(--white); border: 1px solid var(--rule); border-radius: var(--radius); padding: 18px 22px; margin-bottom: 20px; }}
.score-pct {{ font-size: 34px; font-weight: 700; min-width: 70px; font-variant-numeric: tabular-nums; }}
.score-pct.is-autofail {{ color: var(--danger); }}
.score-meta {{ flex: 1; }}
.score-meta-label {{ font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: .08em; color: var(--ink-soft); margin-bottom: 7px; }}
.track {{ height: 6px; background: var(--rule); border-radius: 99px; overflow: hidden; }}
.fill {{ height: 100%; background: var(--accent); border-radius: 99px; transition: width .3s ease; }}
.fill.is-autofail {{ background: var(--danger); }}
.score-frac {{ font-size: 12px; color: var(--ink-soft); margin-top: 5px; }}

/* adjusted score */
.adj-row {{ display: flex; align-items: center; gap: 8px; margin-top: 10px; }}
.adj-row label {{ font-size: 12px; font-weight: 600; color: var(--ink-mid); white-space: nowrap; }}
.adj-input {{
  width: 72px; padding: 5px 8px; font-family: var(--font); font-size: 13px;
  color: var(--ink); border: 1px solid var(--rule); border-radius: var(--radius);
  outline: none; transition: border-color .15s; text-align: right;
}}
.adj-input:focus {{ border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-bg); }}
.adj-input:disabled {{ background: var(--surface); color: var(--ink-soft); cursor: not-allowed; }}
.adj-reset {{ background: none; border: none; cursor: pointer; font-size: 12px; color: var(--ink-soft); padding: 2px 4px; border-radius: 3px; transition: color .12s; display: none; }}
.adj-reset:hover {{ color: var(--danger); }}
.adj-reset.visible {{ display: inline; }}

/* autofail banner */
.autofail-banner {{ display: none; align-items: center; gap: 8px; padding: 11px 14px; background: var(--danger-bg); border: 1px solid #f5c6c6; border-radius: var(--radius); margin-bottom: 20px; font-size: 13px; color: var(--danger); font-weight: 500; }}
.autofail-banner.visible {{ display: flex; }}

/* criteria table */
.criteria-table {{ width: 100%; border-collapse: collapse; }}
.criteria-table thead tr {{ border-bottom: 2px solid var(--rule); }}
.criteria-table thead th {{ padding: 10px 12px; font-size: 12px; font-weight: 600; color: var(--ink-mid); text-align: center; white-space: nowrap; }}
.criteria-table thead th.col-crit {{ text-align: left; min-width: 160px; }}
.criteria-table thead th.col-comment {{ text-align: left; min-width: 130px; }}
.criteria-table tbody tr {{ border-bottom: 1px solid var(--rule); }}
.criteria-table tbody tr:last-child {{ border-bottom: none; }}
.crit-name {{ padding: 14px 12px; font-size: 14px; vertical-align: top; }}
.crit-name code {{ font-family: var(--mono); font-size: 13px; }}

/* option cells */
.opt-cell {{ padding: 14px 8px; text-align: center; vertical-align: top; }}
.opt-unavailable {{ color: var(--rule); font-size: 18px; line-height: 36px; }}
.opt-label {{ display: inline-flex; flex-direction: column; align-items: center; cursor: pointer; }}
.opt-label input[type="radio"] {{ display: none; }}
.opt-btn {{
  display: flex; align-items: center; justify-content: center;
  width: 38px; height: 38px; border-radius: var(--radius);
  border: 1.5px solid var(--rule); background: var(--white);
  font-size: 12px; font-weight: 600; color: var(--ink-mid);
  transition: background .12s, border-color .12s, color .12s;
  user-select: none;
}}
.opt-label:hover .opt-btn {{ border-color: #9ca3af; background: var(--surface); }}
.opt-label input:checked + .opt-btn {{ box-shadow: 0 0 0 3px var(--accent-bg); }}

/* per-type checked colours */
.opt-full    input:checked + .opt-btn {{ border-color: var(--success); background: var(--success-bg); color: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }}
.opt-points  input:checked + .opt-btn {{ border-color: var(--accent);  background: var(--accent-bg);  color: var(--accent);  box-shadow: 0 0 0 3px var(--accent-bg); }}
.opt-na      input:checked + .opt-btn {{ border-color: #9ca3af; background: var(--na-bg); color: var(--na-color); box-shadow: 0 0 0 3px var(--na-bg); }}
.opt-autofail input:checked + .opt-btn {{ border-color: var(--danger); background: var(--danger-bg); color: var(--danger); box-shadow: 0 0 0 3px var(--danger-bg); }}

/* comment cell */
.comment-cell {{ padding: 14px 12px 14px 0; vertical-align: top; }}
.comment-toggle {{ background: none; border: none; cursor: pointer; font-size: 12px; color: var(--ink-soft); padding: 4px 0; transition: color .12s; }}
.comment-toggle:hover {{ color: var(--accent); }}
.comment-area {{ margin-top: 6px; }}
.comment-area textarea {{
  width: 100%; min-height: 58px; padding: 7px 9px;
  font-family: var(--font); font-size: 13px; color: var(--ink);
  border: 1px solid var(--rule); border-radius: var(--radius);
  resize: vertical; outline: none; transition: border-color .15s;
}}
.comment-area textarea:focus {{ border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-bg); }}
.comment-area textarea::placeholder {{ color: var(--ink-soft); }}

/* actions */
.actions {{ display: flex; gap: 10px; justify-content: flex-end; }}
.btn {{ display: inline-flex; align-items: center; padding: 9px 18px; border-radius: var(--radius); font-family: var(--font); font-size: 14px; font-weight: 500; cursor: pointer; border: 1.5px solid transparent; transition: background .12s, border-color .12s; }}
.btn-secondary {{ background: var(--white); border-color: var(--rule); color: var(--ink-mid); }}
.btn-secondary:hover {{ border-color: #bbb; background: var(--surface); }}
.btn-primary {{ background: var(--accent); color: var(--white); border-color: var(--accent); }}
.btn-primary:hover {{ background: #1d4ed8; }}

@media (prefers-reduced-motion: reduce) {{ *, *::before, *::after {{ transition: none !important; }} }}
</style>
</head>
<body>

<header class="page-header">
  <span class="wordmark">QAMS</span>
  <span class="sep">|</span>
  <span class="page-title">Review Form</span>
</header>

<main class="shell">

  <p class="section-label">Review details</p>
  <div class="card" style="margin-bottom:20px">
    <div class="card-body">
      <div class="meta-grid">
        <div class="field">
          <label for="agent">Agent</label>
          <select id="agent" name="agent">
            <option value="" disabled selected>Select agent…</option>
            {agent_options}
          </select>
        </div>
        <div class="field">
          <label for="reviewer">Reviewer</label>
          <input type="text" id="reviewer" name="reviewer" placeholder="Your name">
        </div>
        <div class="field">
          <label for="date">Date</label>
          <input type="date" id="date" name="date">
        </div>
      </div>
    </div>
  </div>

  <div class="score-bar-card">
    <div class="score-pct" id="score-pct">—</div>
    <div class="score-meta">
      <div class="score-meta-label">Score</div>
      <div class="track"><div class="fill" id="score-fill" style="width:0%"></div></div>
      <div class="score-frac" id="score-frac">Answer all criteria to see score</div>
      <div class="adj-row">
        <label for="adj-score">Adjusted %</label>
        <input type="number" id="adj-score" class="adj-input" min="0" max="100"
               placeholder="—" disabled oninput="onAdjInput()">
        <button type="button" class="adj-reset" id="adj-reset"
                onclick="clearAdj()" title="Clear adjustment">&#x2715;</button>
      </div>
    </div>
  </div>

  <div class="autofail-banner" id="autofail-banner">
    &#9888; Autofail — this review scores 0% regardless of other selections.
  </div>

  <p class="section-label">Criteria</p>
  <div class="card">
    <table class="criteria-table">
      <thead>
        <tr>
          <th class="col-crit">Criterion</th>
          {option_headers}
          <th class="col-comment">Comment</th>
        </tr>
      </thead>
      <tbody>
        {criterion_rows}
      </tbody>
    </table>
  </div>

  <div class="actions">
    <button type="button" class="btn btn-secondary" onclick="resetForm()">Clear</button>
    <button type="button" class="btn btn-primary" onclick="saveReview()">Save review</button>
  </div>

</main>

<script>
// Scorecard structure for client-side score calculation.
// Mirrors the Rust data model: criteria keyed by name, each with a map of
// option name → score type. Criterion and option order follow the CSV.
const CRITERIA = [{criteria_js}];
const OPTION_ORDER = [{option_order_js}];

function maxPts(crit) {{
  return Math.max(0, ...Object.values(crit.options)
    .filter(o => o.type === "points").map(o => o.value));
}}

function recalc() {{
  let num = 0, denom = 0, autofail = false, answered = 0;
  for (const crit of CRITERIA) {{
    const sel = document.querySelector(`input[name="${{crit.name}}"]:checked`);
    if (!sel) continue;
    answered++;
    const opt = crit.options[sel.value];
    if (opt.type === "autofail") {{ autofail = true; break; }}
    if (opt.type === "na") continue;
    denom += maxPts(crit);
    num   += opt.value;
  }}

  const pctEl  = document.getElementById("score-pct");
  const fillEl = document.getElementById("score-fill");
  const fracEl = document.getElementById("score-frac");
  const banner = document.getElementById("autofail-banner");

  if (answered === 0) {{
    pctEl.textContent  = "—";
    fillEl.style.width = "0%";
    fracEl.textContent = "Answer all criteria to see score";
    pctEl.className    = "score-pct";
    fillEl.className   = "fill";
    banner.classList.remove("visible");
    document.getElementById("adj-score").disabled = true;
    clearAdj();
    return;
  }}
  if (autofail) {{
    pctEl.textContent  = "0%";
    fillEl.style.width = "0%";
    fracEl.textContent = "Autofail";
    pctEl.className    = "score-pct is-autofail";
    fillEl.className   = "fill is-autofail";
    banner.classList.add("visible");
    document.getElementById("adj-score").disabled = false;
    clearAdj();
    return;
  }}
  banner.classList.remove("visible");
  const pct = denom > 0 ? Math.round((num / denom) * 100) : 100;
  const remaining = CRITERIA.length - answered;
  pctEl.textContent  = pct + "%";
  fillEl.style.width = pct + "%";
  fracEl.textContent = remaining > 0
    ? `${{num}}/${{denom}} pts · ${{remaining}} criterion${{remaining > 1 ? "a" : "ion"}} unanswered`
    : `${{num}}/${{denom}} pts`;
  pctEl.className  = "score-pct";
  fillEl.className = "fill";
  document.getElementById("adj-score").disabled = false;
  clearAdj();
}}

function onAdjInput() {{
  const el  = document.getElementById("adj-score");
  const rst = document.getElementById("adj-reset");
  // Clamp to 0–100
  if (el.value !== "" && (el.value < 0 || el.value > 100)) {{
    el.value = Math.min(100, Math.max(0, el.value));
  }}
  rst.classList.toggle("visible", el.value !== "");
}}

function clearAdj() {{
  const el = document.getElementById("adj-score");
  el.value = "";
  document.getElementById("adj-reset").classList.remove("visible");
}}

function toggleComment(uid) {{
  const area = document.getElementById("comment-" + uid);
  const btn  = area.previousElementSibling;
  const open = area.hasAttribute("hidden");
  if (open) {{ area.removeAttribute("hidden"); area.querySelector("textarea").focus(); }}
  else       {{ area.setAttribute("hidden", ""); }}
  btn.setAttribute("aria-expanded", open);
  btn.textContent = open ? "− comment" : "+ comment";
}}

function resetForm() {{
  document.querySelectorAll("input[type=radio]").forEach(r => r.checked = false);
  document.querySelectorAll("textarea").forEach(t => t.value = "");
  document.querySelectorAll(".comment-area:not([hidden])").forEach(a => {{
    a.setAttribute("hidden", "");
    a.previousElementSibling.setAttribute("aria-expanded", "false");
    a.previousElementSibling.textContent = "+ comment";
  }});
  clearAdj();
  document.getElementById("adj-score").disabled = true;
  recalc();
}}

function saveReview() {{
  const agent    = document.getElementById("agent").value;
  const reviewer = document.getElementById("reviewer").value.trim();
  const date     = document.getElementById("date").value;
  if (!agent || !reviewer || !date) {{
    alert("Please fill in agent, reviewer, and date before saving.");
    return;
  }}
  const unanswered = CRITERIA.filter(c =>
    !document.querySelector(`input[name="${{c.name}}"]:checked`)
  );
  if (unanswered.length) {{
    alert("Please answer all criteria before saving.\nUnanswered: " +
          unanswered.map(c => c.name).join(", "));
    return;
  }}
  const selections = {{}};
  const comments   = {{}};
  for (const crit of CRITERIA) {{
    selections[crit.name] = document.querySelector(`input[name="${{crit.name}}"]:checked`).value;
    const uid = crit.name.replace(/\W/g, "_");
    const txt = document.querySelector(`#comment-${{uid}} textarea`).value.trim();
    if (txt) comments[crit.name] = txt;
  }}
  const adjEl  = document.getElementById("adj-score");
  const score  = parseFloat(document.getElementById("score-pct").textContent) || 0;
  const review = {{ agent, reviewer, date, selections, comments, score }};
  if (adjEl.value !== "") review.adj_score = parseFloat(adjEl.value);
  const filename = agent.replace(/\s+/g, "_") + "_" + date + ".json";
  const blob = new Blob([JSON.stringify(review, null, 2)], {{ type: "application/json" }});
  const url  = URL.createObjectURL(blob);
  const a    = document.createElement("a");
  a.href = url; a.download = filename; a.click();
  URL.revokeObjectURL(url);
}}

document.addEventListener("DOMContentLoaded", () => {{
  document.getElementById("date").valueAsDate = new Date();
}});
</script>
</body>
</html>"#,
        option_headers   = option_headers,
        criterion_rows   = criterion_rows,
        agent_options    = agent_options,
        criteria_js      = criteria_js(scorecard),
        option_order_js  = scorecard.option_order()
            .iter()
            .map(|o| format!("\"{}\"", escape_js_in_html(o)))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Serialises the scorecard to a JS array literal for the score calculator.
fn criteria_js(scorecard: &Scorecard) -> String {
    scorecard.criterion_order()
        .iter()
        .map(|crit_name| {
            let criterion = scorecard.criterion(crit_name)
                .expect("criterion_order must be consistent with criteria");
            let max_pts = criterion.max_points();

            let options_js: String = criterion.options()
                .map(|(opt_name, score)| {
                    let val = match score {
                        CriterionScore::Points(p) =>
                            format!("{{type:\"points\",value:{}}}", p),
                        CriterionScore::NotApplicable =>
                            "{type:\"na\"}".to_string(),
                        CriterionScore::Autofail =>
                            "{type:\"autofail\"}".to_string(),
                    };
                    format!("\"{}\":{}", escape_js_in_html(opt_name), val)
                })
                .collect::<Vec<_>>()
                .join(",");

            format!(
                "{{name:\"{}\",maxPts:{},options:{{{}}}}}",
                escape_js_in_html(crit_name),
                max_pts,
                options_js,
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Escapes the five XML special characters for use in HTML attribute values
/// and text content.
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

/// Escapes characters that would break a JS string literal (double-quoted).
fn escape_js(s: &str) -> String {
    s.chars().map(|c| match c {
        '"'  => "\\\"".to_string(),
        '\\' => "\\\\".to_string(),
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        c    => c.to_string(),
    }).collect()
}

/// Escapes a value for use inside a double-quoted JS string that is itself
/// embedded in an HTML document. HTML-escaping is applied first (so `<` and
/// `&` are safe for the HTML parser), then JS-escaping (so `"` and `\` are
/// safe inside the string literal). The order matters: JS-escaping first
/// would turn `&` into `&` which HTML-escaping would then incorrectly expand.
fn escape_js_in_html(s: &str) -> String {
    escape_js(&escape_html(s))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use super::*;

    const VSC1_CSV: &str = include_str!("../../qams_core/test_artifacts/vsc1.csv");

    const TEST_AGENTS: &[&[&str]] = &[
        &["Alice Nguyen", "Team A"],
        &["Ben Carter"],
    ];

    #[test]
    fn from_csv_vsc1() {
        use std::collections::HashMap;
        let sc = parse_scorecard_csv(VSC1_CSV).expect("parse should succeed");

        // crit1: YES=1, NO=0, N/A=NotApplicable, FYI=1
        // crit2: YES=1, NO=0  (N/A and FYI absent)

        let sel_perfect: HashMap<String, String> = HashMap::from([
            ("crit1".into(), "YES".into()),
            ("crit2".into(), "YES".into()),
        ]);
        assert_eq!(sc.get_score(&sel_perfect), 1.0);

        let sel_zero: HashMap<String, String> = HashMap::from([
            ("crit1".into(), "NO".into()),
            ("crit2".into(), "NO".into()),
        ]);
        assert_eq!(sc.get_score(&sel_zero), 0.0);

        let sel_na: HashMap<String, String> = HashMap::from([
            ("crit1".into(), "N/A".into()),
            ("crit2".into(), "YES".into()),
        ]);
        assert_eq!(sc.get_score(&sel_na), 1.0);

        let sel_fyi: HashMap<String, String> = HashMap::from([
            ("crit1".into(), "FYI".into()),
            ("crit2".into(), "NO".into()),
        ]);
        assert_eq!(sc.get_score(&sel_fyi), 0.5);
    }

    #[test]
    fn from_csv_duplicate_option_error() {
        let csv = ",A,A\ncrit1,1,0\n";
        assert!(parse_scorecard_csv(csv).is_err());
    }

    #[test]
    fn from_csv_duplicate_criterion_error() {
        let csv = ",YES,NO\ncrit1,1,0\ncrit1,1,0\n";
        assert!(parse_scorecard_csv(csv).is_err());
    }

    #[test]
    fn from_csv_invalid_cell_error() {
        let csv = ",YES,NO\ncrit1,1,BAD\n";
        assert!(parse_scorecard_csv(csv).is_err());
    }

    // ── render_scorecard_html tests ─────────────────────────────────────

    #[test]
    fn to_html_is_valid_html_document() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn to_html_preserves_criterion_order() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        let pos_crit1 = html.find("crit1").expect("crit1 missing");
        let pos_crit2 = html.find("crit2").expect("crit2 missing");
        assert!(pos_crit1 < pos_crit2, "crit1 should appear before crit2");
    }

    #[test]
    fn to_html_preserves_option_order() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        let pos_yes = html.find(">YES<").expect("YES header missing");
        let pos_no  = html.find(">NO<").expect("NO header missing");
        let pos_na  = html.find(">N/A<").expect("N/A header missing");
        let pos_fyi = html.find(">FYI<").expect("FYI header missing");
        assert!(pos_yes < pos_no,  "YES should come before NO");
        assert!(pos_no  < pos_na,  "NO should come before N/A");
        assert!(pos_na  < pos_fyi, "N/A should come before FYI");
    }

    #[test]
    fn to_html_unavailable_options_render_as_dash() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        assert!(
            html.contains("opt-unavailable"),
            "unavailable options should use the opt-unavailable class"
        );
    }

    #[test]
    fn to_html_contains_criteria_js_block() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        assert!(html.contains("\"crit1\""), "criteria JS block should include crit1");
        assert!(html.contains("\"crit2\""), "criteria JS block should include crit2");
    }

    #[test]
    fn to_html_escapes_special_chars() {
        let csv = ",<YES>&,\"opt\"\ncrit<1>,1,0\n";
        let sc = parse_scorecard_csv(csv).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        assert!(!html.contains("<YES>"),   "raw < > in option name should be escaped");
        assert!(!html.contains("crit<1>"), "raw < > in criterion name should be escaped");
        assert!(html.contains("&lt;"),     "escaped < should appear as &lt;");
    }

    #[test]
    fn to_html_agent_options() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let agents: &[&[&str]] = &[
            &["Alice Nguyen", "Team A"],
            &["Ben Carter"],
            &["Clara & <Singh>"],
        ];
        let html = render_scorecard_html(&sc, agents);
        assert!(html.contains("value=\"Alice Nguyen\""),    "identifier rendered as option value");
        assert!(html.contains("data-meta-0=\"Team A\""),    "metadata stored as data attribute");
        assert!(html.contains("Ben Carter"),                "agent with no metadata renders");
        assert!(!html.contains("Clara & <Singh>"),          "raw special chars must be escaped");
        assert!(html.contains("Clara &amp; &lt;Singh&gt;"), "agent name HTML-escaped");
    }

    #[test]
    fn sanity_check_vsc1() {
        let sc = parse_scorecard_csv(VSC1_CSV).unwrap();
        let html = render_scorecard_html(&sc, TEST_AGENTS);
        fs::write("../qams_core/test_artifacts/vsc1.html", html).unwrap();
    }
}
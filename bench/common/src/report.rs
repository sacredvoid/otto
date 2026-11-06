use crate::score::Kind;
use crate::stats::percentile;
use std::fmt::Write as _;
use std::path::Path;

pub struct Row {
    pub model: String,
    pub device: String,
    pub id: String,
    pub kind: Kind,
    pub ms: u128,
    pub text: String,
    pub wer: f64,
    pub hit: bool,
}

fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write `rows.csv` and `summary.md` (per model x device) into `out_dir`.
/// `mem` holds (model, device, peak MB). Hit % and mean WER cover speech rows only.
pub fn write_reports(
    rows: &[Row],
    mem: &[(String, String, u64)],
    out_dir: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let mut c = String::from("model,device,id,kind,ms,wer,hit,text\n");
    for r in rows {
        let kind = if r.kind == Kind::Silence { "silence" } else { "speech" };
        let _ = writeln!(
            c,
            "{},{},{},{},{},{:.4},{},{}",
            csv(&r.model),
            csv(&r.device),
            csv(&r.id),
            kind,
            r.ms,
            r.wer,
            r.hit,
            csv(&r.text)
        );
    }
    std::fs::write(out_dir.join("rows.csv"), c).map_err(|e| e.to_string())?;

    let mut groups: Vec<(&str, &str)> = Vec::new();
    for r in rows {
        let g = (r.model.as_str(), r.device.as_str());
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let mut s = String::from(
        "| model | device | entity hit % | mean WER | p50 ms | p95 ms | silence non-empty | peak MB |\n\
         |---|---|---|---|---|---|---|---|\n",
    );
    for (model, device) in groups {
        let g: Vec<&Row> = rows
            .iter()
            .filter(|r| r.model == model && r.device == device)
            .collect();
        let speech: Vec<&&Row> = g.iter().filter(|r| r.kind == Kind::Speech).collect();
        let (hit_pct, mean_wer) = if speech.is_empty() {
            (0.0, 0.0)
        } else {
            let n = speech.len() as f64;
            (
                100.0 * speech.iter().filter(|r| r.hit).count() as f64 / n,
                speech.iter().map(|r| r.wer).sum::<f64>() / n,
            )
        };
        let ms: Vec<u128> = g.iter().map(|r| r.ms).collect();
        let silent = g
            .iter()
            .filter(|r| r.kind == Kind::Silence && !r.text.trim().is_empty())
            .count();
        let peak = mem
            .iter()
            .find(|(m, d, _)| m == model && d == device)
            .map(|(_, _, mb)| *mb)
            .unwrap_or(0);
        let _ = writeln!(
            s,
            "| {model} | {device} | {hit_pct:.1} | {mean_wer:.3} | {} | {} | {silent} | {peak} |",
            percentile(&ms, 50.0),
            percentile(&ms, 95.0)
        );
    }
    std::fs::write(out_dir.join("summary.md"), s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_summary_with_silence_count() {
        let row = |id: &str, kind, text: &str, hit| Row {
            model: "m".into(),
            device: "cpu".into(),
            id: id.into(),
            kind,
            ms: 100,
            text: text.into(),
            wer: 0.0,
            hit,
        };
        let rows = vec![
            row("a", Kind::Speech, "hi, there", true),
            row("s", Kind::Silence, "thanks", false),
        ];
        let dir = std::env::temp_dir().join("stt_bench_report_test");
        write_reports(&rows, &[("m".into(), "cpu".into(), 7)], &dir).unwrap();
        let md = std::fs::read_to_string(dir.join("summary.md")).unwrap();
        assert!(md.contains("| m | cpu | 100.0 | 0.000 | 100 | 100 | 1 | 7 |"), "{md}");
        assert!(std::fs::read_to_string(dir.join("rows.csv")).unwrap().contains("\"hi, there\""));
    }
}

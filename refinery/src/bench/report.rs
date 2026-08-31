//! What a benchmark run observed, in a form that can be committed as evidence.
//!
//! A report renders twice: JSON for a machine to diff two runs, and Markdown
//! for the reader deciding whether a change cost anything. Both carry the same
//! facts — the second is a rendering of the first.
//!
//! Only measured facts are stored. Throughput is derived on the way out
//! ([`CaseResult::records_per_second`] and friends) rather than recorded
//! beside the figures it comes from, so a committed report can never disagree
//! with itself.

use serde::{Deserialize, Serialize};

use super::BenchError;
use crate::manifest::{RecordGeometry, ToolIdentity};
use crate::soak::HostFacts;

/// Bytes in a gibibyte — the unit the issue asks input throughput in.
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Bytes in a mebibyte, for the sizes a reader reads.
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// The synthetic corpus a benchmark was run over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusFacts {
    /// Corpus files written.
    pub shards: usize,
    /// Records in each of them.
    pub records_per_shard: usize,
    /// Total bytes across the corpus.
    pub bytes: u64,
}

/// What one measured case cost, and what it published.
///
/// `elapsed_ms` is the fastest of the repeats — the run least disturbed by
/// whatever else the host was doing — and `peak_rss_kib` is the worst of them,
/// because a peak is a ceiling and the highest one observed is the honest
/// figure to quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseResult {
    /// The label the case was measured under.
    pub label: String,
    /// The command line it stands for.
    pub transform: String,
    /// How many times it was run.
    pub repeats: usize,
    /// Wall-clock of the fastest run, in milliseconds.
    pub elapsed_ms: u64,
    /// Highest resident set size sampled across the runs, in KiB.
    pub peak_rss_kib: Option<u64>,
    /// How that peak was obtained.
    pub peak_rss_method: String,
    /// Bytes read from the source corpus.
    pub input_bytes: u64,
    /// Bytes published.
    pub output_bytes: u64,
    /// Records read.
    pub records_read: u64,
    /// Records published.
    pub records_written: u64,
}

impl CaseResult {
    /// Records read a second.
    #[must_use]
    pub fn records_per_second(&self) -> f64 {
        if self.elapsed_ms == 0 {
            return 0.0;
        }
        self.records_read as f64 * 1000.0 / self.elapsed_ms as f64
    }

    /// Source corpus read a second, in GiB.
    #[must_use]
    pub fn input_gib_per_second(&self) -> f64 {
        if self.elapsed_ms == 0 {
            return 0.0;
        }
        (self.input_bytes as f64 / BYTES_PER_GIB) * 1000.0 / self.elapsed_ms as f64
    }

    /// Published bytes as a share of the bytes read.
    #[must_use]
    pub fn output_ratio(&self) -> f64 {
        if self.input_bytes == 0 {
            return 0.0;
        }
        self.output_bytes as f64 / self.input_bytes as f64
    }

    /// Published size in MiB.
    #[must_use]
    pub fn output_mib(&self) -> f64 {
        self.output_bytes as f64 / BYTES_PER_MIB
    }

    /// The row this case renders as in the evidence table.
    fn to_row(&self) -> String {
        format!(
            "| {} | `{}` | {} | {:.2} | {:.0} | {} | {:.1} | {:.3} |\n",
            self.label,
            self.transform,
            self.elapsed_ms,
            self.input_gib_per_second(),
            self.records_per_second(),
            self.peak_rss_kib
                .map_or_else(|| "not sampled".to_string(), |kib| kib.to_string()),
            self.output_mib(),
            self.output_ratio(),
        )
    }
}

/// The Deno sampler measured beside Refinery, over the same corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The label of the Refinery case it mirrors.
    pub mirrors: String,
    /// What it cost, measured exactly as the Refinery cases were.
    pub result: CaseResult,
}

/// Everything one benchmark run observed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    /// The build that was measured.
    pub tool: ToolIdentity,
    /// The host it was measured on.
    pub host: HostFacts,
    /// The record shape every case read the corpus with.
    pub record_shape: RecordGeometry,
    /// The corpus every case read.
    pub corpus: CorpusFacts,
    /// How many times each case was run.
    pub repeats: usize,
    /// Each measured case, in the order the suite ran them.
    pub cases: Vec<CaseResult>,
    /// The Deno sampler over the same corpus, when it was measured.
    pub reference: Option<Reference>,
}

impl BenchReport {
    /// The case measured under `label`, when the run measured it.
    #[must_use]
    pub fn case(&self, label: &str) -> Option<&CaseResult> {
        self.cases.iter().find(|case| case.label == label)
    }

    /// How many times the records a second Refinery sampled at exceed the Deno
    /// sampler's, when both were measured in this run.
    #[must_use]
    pub fn sample_speedup(&self) -> Option<f64> {
        let reference = self.reference.as_ref()?;
        let mirrored = self.case(&reference.mirrors)?;
        let deno = reference.result.records_per_second();
        (deno > 0.0).then(|| mirrored.records_per_second() / deno)
    }

    /// How much of the Deno sampler's peak memory Refinery needed.
    #[must_use]
    pub fn peak_rss_share(&self) -> Option<f64> {
        let reference = self.reference.as_ref()?;
        let mirrored = self.case(&reference.mirrors)?;
        let deno = reference.result.peak_rss_kib? as f64;
        let refinery = mirrored.peak_rss_kib? as f64;
        (deno > 0.0).then(|| refinery / deno)
    }

    /// Holds this run to a floor against the Deno sampler measured beside it.
    ///
    /// This is the gate CI enforces: both samplers ran on the same runner in
    /// the same job, so their ratio survives a noisy host in a way an absolute
    /// throughput never does.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Config`] when this run measured no reference to
    /// gate against — a gate with nothing to compare cannot pass — and
    /// [`BenchError::Regression`] when the measured speedup is below
    /// `minimum`.
    pub fn check_speedup(&self, minimum: f64) -> Result<f64, BenchError> {
        let speedup = self.sample_speedup().ok_or_else(|| {
            BenchError::config(
                "a speedup gate needs the Deno sampler measured beside Refinery; this run measured none",
            )
        })?;
        if speedup < minimum {
            return Err(BenchError::regression(format!(
                "Refinery sampled at {speedup:.2}× the Deno sampler, below the {minimum:.2}× gate"
            )));
        }
        Ok(speedup)
    }

    /// The report as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Json`] when the report cannot be encoded.
    pub fn to_json(&self) -> Result<String, BenchError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// The report as Markdown, for committing beside a pull request.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Refinery benchmark\n\n");
        out.push_str(&format!(
            "- **tool** — `{} {}`\n- **host** — `{}` / `{}` ({} cpus, {} family)\n",
            self.tool.name,
            self.tool.version,
            self.host.os,
            self.host.arch,
            self.host
                .cpu_count
                .map_or_else(|| "unknown".to_string(), |count| count.to_string()),
            self.host.family,
        ));
        out.push_str(&format!(
            "- **corpus** — {} shards × {} records ({:.1} MiB), {} bytes a record\n",
            self.corpus.shards,
            self.corpus.records_per_shard,
            self.corpus.bytes as f64 / BYTES_PER_MIB,
            self.record_shape.bytes_per_record,
        ));
        out.push_str(&format!(
            "- **repeats** — {} (the fastest run of each case is reported; peak RSS is the worst)\n\n",
            self.repeats,
        ));

        out.push_str(
            "| case | transform | wall-clock ms | input GiB/s | records/s | peak RSS KiB | output MiB | output/input |\n",
        );
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for case in &self.cases {
            out.push_str(&case.to_row());
        }
        if let Some(reference) = &self.reference {
            out.push_str(&reference.result.to_row());
        }
        out.push('\n');

        if let Some(speedup) = self.sample_speedup() {
            out.push_str(&format!(
                "Refinery reads {speedup:.1}× the records a second the Deno sampler does",
            ));
            match self.peak_rss_share() {
                Some(share) => out.push_str(&format!(", at {share:.2}× its peak RSS.\n")),
                None => out.push_str(".\n"),
            }
        }
        out.push_str(&format!(
            "\nPeak RSS is sampled — method `{}`.\n",
            self.cases
                .first()
                .map_or("none", |case| case.peak_rss_method.as_str()),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(label: &str, elapsed_ms: u64) -> CaseResult {
        CaseResult {
            label: label.to_string(),
            transform: "sample --rate 0.05".to_string(),
            repeats: 3,
            elapsed_ms,
            peak_rss_kib: Some(12_000),
            peak_rss_method: "proc-vmhwm".to_string(),
            input_bytes: 2 * 1024 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            records_read: 200_000,
            records_written: 10_000,
        }
    }

    fn report(reference: Option<Reference>) -> BenchReport {
        BenchReport {
            tool: ToolIdentity::current(),
            host: HostFacts::detect(),
            record_shape: crate::corpus::RecordShape::new(2, 1)
                .expect("valid shape")
                .into(),
            corpus: CorpusFacts {
                shards: 2,
                records_per_shard: 100_000,
                bytes: 2 * 1024 * 1024 * 1024,
            },
            repeats: 3,
            cases: vec![case("sample", 1_000)],
            reference,
        }
    }

    fn deno(elapsed_ms: u64, peak_rss_kib: Option<u64>) -> Reference {
        let mut result = case("typescript", elapsed_ms);
        result.peak_rss_kib = peak_rss_kib;
        Reference {
            mirrors: "sample".to_string(),
            result,
        }
    }

    #[test]
    fn derives_throughput_from_the_facts_it_measured() {
        let case = case("sample", 1_000);

        assert!((case.records_per_second() - 200_000.0).abs() < f64::EPSILON);
        assert!((case.input_gib_per_second() - 2.0).abs() < 1e-9);
        assert!((case.output_mib() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reports_no_throughput_rather_than_dividing_by_zero() {
        let case = case("sample", 0);

        assert!((case.records_per_second() - 0.0).abs() < f64::EPSILON);
        assert!((case.input_gib_per_second() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compares_the_two_samplers_measured_in_one_run() {
        let report = report(Some(deno(4_000, Some(160_000))));

        let speedup = report.sample_speedup().expect("both were measured");
        assert!((speedup - 4.0).abs() < 1e-9, "speedup was {speedup}");
        let share = report.peak_rss_share().expect("both peaks were sampled");
        assert!((share - 0.075).abs() < 1e-9, "share was {share}");
        assert!(report.to_markdown().contains("4.0×"));
    }

    #[test]
    fn the_gate_fails_loud_below_its_floor_and_passes_above_it() {
        let report = report(Some(deno(4_000, Some(160_000))));

        let error = report
            .check_speedup(8.0)
            .expect_err("a 4× run must not clear an 8× gate");
        assert!(matches!(error, BenchError::Regression { .. }), "{error:?}");
        assert!((report.check_speedup(2.0).expect("clears a 2× gate") - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_gate_with_nothing_to_compare_against_cannot_pass() {
        let report = report(None);

        assert!(report.sample_speedup().is_none());
        let error = report
            .check_speedup(1.5)
            .expect_err("an unmeasured comparison is not a pass");
        assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
    }

    #[test]
    fn states_plainly_when_memory_was_not_sampled() {
        let mut unsampled = report(Some(deno(4_000, None)));
        unsampled.cases[0].peak_rss_kib = None;

        assert!(unsampled.peak_rss_share().is_none());
        assert!(unsampled.to_markdown().contains("not sampled"));
    }
}

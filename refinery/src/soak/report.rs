//! What a soak run produced, in a form that can be committed as evidence.
//!
//! A report renders twice: JSON for a machine to diff two hosts, and Markdown
//! for the reviewer who has to decide whether the cut-over is justified. Both
//! carry the same facts — the second is a rendering of the first, never a
//! summary that quietly drops a number.

use serde::{Deserialize, Serialize};

use super::{HostFacts, PublishedCorpus, RunMeasurement, SoakError};
use crate::manifest::{RecordGeometry, ToolIdentity};

/// The synthetic corpus a soak was run over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusFacts {
    /// Corpus files written.
    pub shards: usize,
    /// Records in each of them.
    pub records_per_shard: usize,
    /// Total bytes across the corpus.
    pub bytes: u64,
}

/// One measured sampling round and the corpus it published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoakRound {
    /// Which round this was, counting from one.
    pub round: usize,
    /// What the run cost.
    pub measurement: RunMeasurement,
    /// The corpus it published, re-verified from the outside.
    pub published: PublishedCorpus,
}

/// Evidence that a failed run does not damage the published corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomicPublicationEvidence {
    /// The exit code of the deliberately failed run.
    pub failed_run_exit_code: Option<i32>,
    /// Whether the previously published corpus was byte-identical afterwards.
    pub previous_corpus_intact: bool,
    /// Staging or aside directories the failed run left behind.
    pub scratch_left_behind: usize,
}

/// Evidence that NEAT-AI still consumes what Refinery published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerCheck {
    /// Whether `evolveDir` opened and used the published corpus.
    pub consumed: bool,
    /// The line the consumer harness reported.
    pub summary: String,
}

/// Everything one soak run observed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoakReport {
    /// The build that was soaked.
    pub tool: ToolIdentity,
    /// The host it was soaked on.
    pub host: HostFacts,
    /// The record shape every run used.
    pub record_shape: RecordGeometry,
    /// The sampling rate every run used.
    pub rate: f64,
    /// The corpus the runs read.
    pub corpus: CorpusFacts,
    /// Each measured round.
    pub rounds: Vec<SoakRound>,
    /// The Deno sampler over the same corpus, when it was measured.
    pub reference: Option<RunMeasurement>,
    /// What the Deno sampler published, when it was measured.
    pub reference_records_written: Option<u64>,
    /// The `evolveDir` consumer check, when it was run.
    pub consumer: Option<ConsumerCheck>,
    /// Whether the source corpus was byte-identical after every run.
    pub source_unchanged: bool,
    /// What a deliberately failed run did to the published corpus.
    pub atomic_publication: AtomicPublicationEvidence,
}

impl SoakReport {
    /// The report as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SoakError::Json`] when the report cannot be encoded.
    pub fn to_json(&self) -> Result<String, SoakError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// The report as Markdown, for committing beside a pull request.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Refinery production soak\n\n");
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
            self.corpus.bytes as f64 / (1024.0 * 1024.0),
            self.record_shape.bytes_per_record,
        ));
        out.push_str(&format!(
            "- **rate** — {} (no seed, as production runs)\n\n",
            self.rate
        ));

        out.push_str("| round | elapsed ms | records/s | peak RSS KiB | read | kept |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for round in &self.rounds {
            out.push_str(&format!(
                "| {} | {} | {:.0} | {} | {} | {} |\n",
                round.round,
                round.measurement.elapsed_ms,
                records_per_second(round.published.records_read, round.measurement.elapsed_ms),
                rss(round.measurement.peak_rss_kib),
                round.published.records_read,
                round.published.record_count,
            ));
        }

        if let Some(reference) = &self.reference {
            out.push_str(&format!(
                "| {} | {} | {:.0} | {} | {} | {} |\n",
                reference.label,
                reference.elapsed_ms,
                records_per_second(self.corpus_records(), reference.elapsed_ms),
                rss(reference.peak_rss_kib),
                self.corpus_records(),
                self.reference_records_written
                    .map_or_else(|| "—".to_string(), |kept| kept.to_string()),
            ));
        }
        out.push_str(&format!(
            "\nPeak RSS is sampled — method `{}`.\n\n",
            self.rounds
                .first()
                .map_or("none", |round| round.measurement.peak_rss_method.as_str()),
        ));

        out.push_str("## Invariants\n\n");
        out.push_str(&format!(
            "- **{}** — no source corpus mutation: the source digested identically before and after every run\n",
            verdict(self.source_unchanged),
        ));
        out.push_str(&format!(
            "- **{}** — output geometry: every published corpus re-verified against its own manifest ({} rounds)\n",
            verdict(!self.rounds.is_empty()),
            self.rounds.len(),
        ));
        out.push_str(&format!(
            "- **{}** — atomic publication: a failed run (exit {}) left the published corpus byte-identical and {} scratch directories behind\n",
            verdict(
                self.atomic_publication.previous_corpus_intact
                    && self.atomic_publication.scratch_left_behind == 0
            ),
            self.atomic_publication
                .failed_run_exit_code
                .map_or_else(|| "on a signal".to_string(), |code| code.to_string()),
            self.atomic_publication.scratch_left_behind,
        ));
        match &self.consumer {
            Some(consumer) => out.push_str(&format!(
                "- **{}** — evolveDir consumed the published corpus: `{}`\n",
                verdict(consumer.consumed),
                consumer.summary.trim(),
            )),
            None => out.push_str(
                "- **not run** — evolveDir consumption: the consumer check was not requested on this host\n",
            ),
        }
        out
    }

    /// Records the corpus holds in total.
    fn corpus_records(&self) -> u64 {
        (self.corpus.shards as u64) * (self.corpus.records_per_shard as u64)
    }
}

/// Records a run processed each second.
fn records_per_second(records: u64, elapsed_ms: u64) -> f64 {
    if elapsed_ms == 0 {
        return 0.0;
    }
    records as f64 * 1000.0 / elapsed_ms as f64
}

/// A sampled peak, or a plain statement that it was not sampled.
fn rss(peak: Option<u64>) -> String {
    peak.map_or_else(|| "not sampled".to_string(), |kib| kib.to_string())
}

/// The word an invariant is reported under.
const fn verdict(held: bool) -> &'static str {
    if held {
        "held"
    } else {
        "FAILED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_throughput_only_when_a_run_took_measurable_time() {
        assert!((records_per_second(1_000, 500) - 2_000.0).abs() < f64::EPSILON);
        assert!((records_per_second(1_000, 0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn states_plainly_when_memory_was_not_sampled() {
        assert_eq!(rss(Some(4_096)), "4096");
        assert_eq!(rss(None), "not sampled");
    }

    #[test]
    fn names_a_broken_invariant_loudly() {
        assert_eq!(verdict(true), "held");
        assert_eq!(verdict(false), "FAILED");
    }
}

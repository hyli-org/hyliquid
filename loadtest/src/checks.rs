use anyhow::{bail, Result};

use crate::config::SlaConfig;
use crate::metrics::MetricsSummary;

/// Validate SLA requirements and fail if not met
pub fn validate_sla(summary: &MetricsSummary, sla_config: &SlaConfig) -> Result<()> {
    if !sla_config.enabled {
        tracing::info!("SLA checks disabled");
        return Ok(());
    }

    tracing::info!("Validating SLA requirements...");

    let mut violations = Vec::new();

    // Check P50 latency
    if summary.latencies.p50_ms > sla_config.p50_max_ms {
        violations.push(format!(
            "P50 latency {}ms exceeds maximum {}ms",
            summary.latencies.p50_ms, sla_config.p50_max_ms
        ));
    }

    // Check P95 latency
    if summary.latencies.p95_ms > sla_config.p95_max_ms {
        violations.push(format!(
            "P95 latency {}ms exceeds maximum {}ms",
            summary.latencies.p95_ms, sla_config.p95_max_ms
        ));
    }

    // Check P99 latency
    if summary.latencies.p99_ms > sla_config.p99_max_ms {
        violations.push(format!(
            "P99 latency {}ms exceeds maximum {}ms",
            summary.latencies.p99_ms, sla_config.p99_max_ms
        ));
    }

    // Check error rate
    if summary.error_rate_percent > sla_config.max_error_rate_percent {
        violations.push(format!(
            "Error rate {:.2}% exceeds maximum {:.2}%",
            summary.error_rate_percent, sla_config.max_error_rate_percent
        ));
    }

    // Check minimum fills/executions
    // We approximate this by looking at create_order success count
    let create_order_count: usize = summary
        .endpoints
        .iter()
        .filter(|e| e.name.contains("create_order"))
        .map(|e| e.success_count)
        .sum();

    if (create_order_count as u64) < sla_config.min_fills {
        violations.push(format!(
            "Minimum fills requirement not met: {} < {}",
            create_order_count, sla_config.min_fills
        ));
    }

    // Report results
    if violations.is_empty() {
        println!("\n✅ SLA VALIDATION PASSED");
        println!(
            "  ✓ P50 latency: {}ms <= {}ms",
            summary.latencies.p50_ms, sla_config.p50_max_ms
        );
        println!(
            "  ✓ P95 latency: {}ms <= {}ms",
            summary.latencies.p95_ms, sla_config.p95_max_ms
        );
        println!(
            "  ✓ P99 latency: {}ms <= {}ms",
            summary.latencies.p99_ms, sla_config.p99_max_ms
        );
        println!(
            "  ✓ Error rate: {:.2}% <= {:.2}%",
            summary.error_rate_percent, sla_config.max_error_rate_percent
        );
        println!(
            "  ✓ Minimum fills: {} >= {}",
            create_order_count, sla_config.min_fills
        );
        Ok(())
    } else {
        println!("\n❌ SLA VALIDATION FAILED");
        for violation in &violations {
            println!("  ✗ {violation}");
        }
        bail!(
            "SLA requirements not met: {} violation(s)",
            violations.len()
        );
    }
}

/// Perform pre-flight checks before starting the test
pub fn preflight_checks(base_url: &str) -> Result<()> {
    tracing::info!("Performing pre-flight checks...");

    // Check if URL is reachable (basic check)
    if base_url.is_empty() {
        bail!("Base URL is empty");
    }

    // Warn about production URLs
    if base_url.contains("prod") || base_url.contains("production") {
        tracing::warn!("⚠️  WARNING: Base URL appears to be a production environment!");
        tracing::warn!("⚠️  Load testing against production is strongly discouraged.");
    }

    tracing::info!("✓ Pre-flight checks passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{LatencyMetrics, MetricsSummary};

    fn make_test_summary() -> MetricsSummary {
        MetricsSummary {
            test_start: "2024-01-01T00:00:00Z".to_string(),
            test_duration_secs: 300.0,
            total_requests: 1000,
            successful_requests: 990,
            failed_requests: 10,
            requests_per_second: 3.33,
            error_rate_percent: 1.0,
            latencies: LatencyMetrics {
                min_ms: 5,
                max_ms: 150,
                mean_ms: 45.0,
                p50_ms: 40,
                p90_ms: 80,
                p95_ms: 90,
                p99_ms: 120,
            },
            endpoints: vec![],
        }
    }

    #[test]
    fn test_sla_pass() {
        let summary = make_test_summary();
        let sla = SlaConfig {
            enabled: true,
            p50_max_ms: 50,
            p95_max_ms: 100,
            p99_max_ms: 200,
            max_error_rate_percent: 2.0,
            min_fills: 0,
        };

        assert!(validate_sla(&summary, &sla).is_ok());
    }

    #[test]
    fn test_sla_fail_p95() {
        let summary = make_test_summary();
        let sla = SlaConfig {
            enabled: true,
            p50_max_ms: 50,
            p95_max_ms: 80, // Lower than actual
            p99_max_ms: 200,
            max_error_rate_percent: 2.0,
            min_fills: 0,
        };

        assert!(validate_sla(&summary, &sla).is_err());
    }

    #[test]
    fn test_sla_fail_error_rate() {
        let summary = make_test_summary();
        let sla = SlaConfig {
            enabled: true,
            p50_max_ms: 50,
            p95_max_ms: 100,
            p99_max_ms: 200,
            max_error_rate_percent: 0.5, // Lower than actual
            min_fills: 0,
        };

        assert!(validate_sla(&summary, &sla).is_err());
    }
}

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use goose::metrics::GooseMetrics;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::config::MetricsConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub test_start: String,
    pub test_duration_secs: f64,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub requests_per_second: f64,
    pub error_rate_percent: f64,
    pub latencies: LatencyMetrics,
    pub endpoints: Vec<EndpointMetrics>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatencyMetrics {
    pub min_ms: u64,
    pub max_ms: u64,
    pub mean_ms: f64,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EndpointMetrics {
    pub name: String,
    pub count: usize,
    pub success_count: usize,
    pub error_count: usize,
    pub latency: LatencyMetrics,
}

/// Export metrics to JSON and CSV files
pub fn export_metrics(
    metrics: &GooseMetrics,
    config: &MetricsConfig,
    start_time: DateTime<Utc>,
) -> Result<MetricsSummary> {
    // Create output directory if it doesn't exist
    fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("Failed to create output directory: {}", config.output_dir))?;

    // Calculate summary metrics
    let summary = calculate_summary(metrics, start_time)?;

    // Export JSON summary
    if config.export_json {
        let json_path = Path::new(&config.output_dir).join("summary.json");
        let json_content = serde_json::to_string_pretty(&summary)
            .context("Failed to serialize metrics to JSON")?;
        fs::write(&json_path, json_content)
            .with_context(|| format!("Failed to write JSON file: {json_path:?}"))?;
        tracing::info!("Exported metrics to: {:?}", json_path);
    }

    // Export CSV latencies
    if config.export_csv {
        let csv_path = Path::new(&config.output_dir).join("latencies.csv");
        export_latencies_csv(metrics, &csv_path)?;
        tracing::info!("Exported latencies to: {:?}", csv_path);
    }

    Ok(summary)
}

fn calculate_summary(metrics: &GooseMetrics, start_time: DateTime<Utc>) -> Result<MetricsSummary> {
    let duration = metrics.duration as f64;

    // Aggregate all requests
    let mut total_requests = 0;
    let mut successful_requests = 0;
    let mut failed_requests = 0;

    for request in metrics.requests.values() {
        total_requests += request.success_count + request.fail_count;
        successful_requests += request.success_count;
        failed_requests += request.fail_count;
    }

    let requests_per_second = if duration > 0.0 {
        total_requests as f64 / duration
    } else {
        0.0
    };

    let error_rate_percent = if total_requests > 0 {
        (failed_requests as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    // Calculate global latencies
    let global_latency = calculate_global_latencies(metrics);

    // Calculate per-endpoint metrics
    let mut endpoints = Vec::new();
    for (path, request) in &metrics.requests {
        let percentiles = calculate_percentiles_from_times(&request.raw_data.times);

        let endpoint_latency = LatencyMetrics {
            min_ms: request.raw_data.minimum_time as u64,
            max_ms: request.raw_data.maximum_time as u64,
            mean_ms: if request.raw_data.counter > 0 {
                request.raw_data.total_time as f64 / request.raw_data.counter as f64
            } else {
                0.0
            },
            p50_ms: percentiles.p50,
            p90_ms: percentiles.p90,
            p95_ms: percentiles.p95,
            p99_ms: percentiles.p99,
        };

        endpoints.push(EndpointMetrics {
            name: format!("{:?} {}", request.method, path),
            count: request.success_count + request.fail_count,
            success_count: request.success_count,
            error_count: request.fail_count,
            latency: endpoint_latency,
        });
    }

    Ok(MetricsSummary {
        test_start: start_time.to_rfc3339(),
        test_duration_secs: duration,
        total_requests,
        successful_requests,
        failed_requests,
        requests_per_second,
        error_rate_percent,
        latencies: global_latency,
        endpoints,
    })
}

fn calculate_global_latencies(metrics: &GooseMetrics) -> LatencyMetrics {
    let mut min_ms = usize::MAX;
    let mut max_ms = 0usize;
    let mut total_time = 0usize;
    let mut total_count = 0usize;
    let mut all_times = std::collections::BTreeMap::new();

    // Aggregate min/max/total and merge all times
    for request in metrics.requests.values() {
        if request.raw_data.minimum_time < min_ms {
            min_ms = request.raw_data.minimum_time;
        }
        if request.raw_data.maximum_time > max_ms {
            max_ms = request.raw_data.maximum_time;
        }
        total_time += request.raw_data.total_time;
        total_count += request.raw_data.counter;

        // Merge times from this request
        for (time, count) in &request.raw_data.times {
            *all_times.entry(*time).or_insert(0) += count;
        }
    }

    let mean_ms = if total_count > 0 {
        total_time as f64 / total_count as f64
    } else {
        0.0
    };

    // Calculate percentiles from aggregated times
    let percentiles = calculate_percentiles_from_times(&all_times);

    if min_ms == usize::MAX {
        min_ms = 0;
    }

    LatencyMetrics {
        min_ms: min_ms as u64,
        max_ms: max_ms as u64,
        mean_ms,
        p50_ms: percentiles.p50,
        p90_ms: percentiles.p90,
        p95_ms: percentiles.p95,
        p99_ms: percentiles.p99,
    }
}

struct Percentiles {
    p50: u64,
    p90: u64,
    p95: u64,
    p99: u64,
}

fn calculate_percentiles_from_times(
    times: &std::collections::BTreeMap<usize, usize>,
) -> Percentiles {
    if times.is_empty() {
        return Percentiles {
            p50: 0,
            p90: 0,
            p95: 0,
            p99: 0,
        };
    }

    // Count total samples
    let total: usize = times.values().sum();
    if total == 0 {
        return Percentiles {
            p50: 0,
            p90: 0,
            p95: 0,
            p99: 0,
        };
    }

    // Build cumulative distribution
    let mut cumulative = Vec::new();
    let mut cumsum = 0;
    for (time, count) in times {
        cumsum += count;
        cumulative.push((*time, cumsum));
    }

    // Find percentile values
    let p50_idx = (total as f64 * 0.50) as usize;
    let p90_idx = (total as f64 * 0.90) as usize;
    let p95_idx = (total as f64 * 0.95) as usize;
    let p99_idx = (total as f64 * 0.99) as usize;

    let mut p50 = 0;
    let mut p90 = 0;
    let mut p95 = 0;
    let mut p99 = 0;

    for (time, cumsum) in cumulative {
        if p50 == 0 && cumsum >= p50_idx {
            p50 = time;
        }
        if p90 == 0 && cumsum >= p90_idx {
            p90 = time;
        }
        if p95 == 0 && cumsum >= p95_idx {
            p95 = time;
        }
        if p99 == 0 && cumsum >= p99_idx {
            p99 = time;
        }
    }

    Percentiles {
        p50: p50 as u64,
        p90: p90 as u64,
        p95: p95 as u64,
        p99: p99 as u64,
    }
}

fn export_latencies_csv(metrics: &GooseMetrics, csv_path: &Path) -> Result<()> {
    let mut writer = csv::Writer::from_path(csv_path)
        .with_context(|| format!("Failed to create CSV writer: {csv_path:?}"))?;

    // Write header
    writer.write_record([
        "endpoint", "method", "count", "success", "fail", "min_ms", "mean_ms", "p50_ms", "p95_ms",
        "p99_ms", "max_ms",
    ])?;

    // Write data rows (summary per endpoint)
    for (path, request) in &metrics.requests {
        let mean_ms = if request.raw_data.counter > 0 {
            request.raw_data.total_time as f64 / request.raw_data.counter as f64
        } else {
            0.0
        };

        let percentiles = calculate_percentiles_from_times(&request.raw_data.times);

        writer.write_record(&[
            path.clone(),
            format!("{:?}", request.method),
            (request.success_count + request.fail_count).to_string(),
            request.success_count.to_string(),
            request.fail_count.to_string(),
            request.raw_data.minimum_time.to_string(),
            format!("{mean_ms:.2}"),
            percentiles.p50.to_string(),
            percentiles.p95.to_string(),
            percentiles.p99.to_string(),
            request.raw_data.maximum_time.to_string(),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

/// Print a human-readable summary to console
pub fn print_summary(summary: &MetricsSummary, verbose: bool) {
    println!("\n{}", "=".repeat(80));
    println!("📊 LOAD TEST SUMMARY");
    println!("{}", "=".repeat(80));

    println!("\n⏱️  Test Duration: {:.2}s", summary.test_duration_secs);
    println!("📈 Total Requests: {}", summary.total_requests);
    println!("✅ Successful: {}", summary.successful_requests);
    println!("❌ Failed: {}", summary.failed_requests);
    println!("📊 RPS: {:.2}", summary.requests_per_second);
    println!("💢 Error Rate: {:.2}%", summary.error_rate_percent);

    println!("\n🚀 LATENCY METRICS (milliseconds)");
    println!("{}", "-".repeat(80));
    println!("  Min:  {}ms", summary.latencies.min_ms);
    println!("  Mean: {:.2}ms", summary.latencies.mean_ms);
    println!("  P50:  {}ms", summary.latencies.p50_ms);
    println!("  P90:  {}ms", summary.latencies.p90_ms);
    println!("  P95:  {}ms", summary.latencies.p95_ms);
    println!("  P99:  {}ms", summary.latencies.p99_ms);
    println!("  Max:  {}ms", summary.latencies.max_ms);

    if verbose {
        println!("\n📍 PER-ENDPOINT METRICS");
        println!("{}", "-".repeat(80));
        for endpoint in &summary.endpoints {
            println!("\n  Endpoint: {}", endpoint.name);
            println!(
                "    Count:   {} ({} success, {} errors)",
                endpoint.count, endpoint.success_count, endpoint.error_count
            );
            println!(
                "    Latency: P50={}ms, P95={}ms, P99={}ms",
                endpoint.latency.p50_ms, endpoint.latency.p95_ms, endpoint.latency.p99_ms
            );
        }
    }

    println!("\n{}", "=".repeat(80));
}

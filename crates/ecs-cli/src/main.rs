//! `electrocraft-cli`: run a SPICE netlist headlessly and print CSV.
//!
//! ```text
//! electrocraft-cli circuit.cir            # every analysis card (.op .dc .tran .ac)
//! electrocraft-cli circuit.cir --op       # operating point only
//! electrocraft-cli circuit.cir --report   # also print the import report to stderr
//! ```
//!
//! Netlists use the same importer as the app (`ecs-spice`): elements
//! R C L V I E F G H B D Q M J X, `.model`, `.subckt`, `.param`, `.include`.

use ecs_sim::analysis::{self, Progress};
use ecs_sim::{Method, Options};
use ecs_spice::AnalysisCard;
use std::path::Path;
use std::process::ExitCode;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("electrocraft-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        return Err("usage: electrocraft-cli <netlist.cir> [--op] [--report]".into());
    };
    let imp = ecs_spice::import_file(Path::new(path)).map_err(|e| e.to_string())?;
    if args.iter().any(|a| a == "--report") {
        for r in &imp.report {
            eprintln!("import: {r}");
        }
    }
    let progress = Progress::default();
    let mut cards = imp.analyses.clone();
    if args.iter().any(|a| a == "--op") || cards.is_empty() {
        cards = vec![AnalysisCard::Op];
    }
    for card in cards {
        let result = match card {
            AnalysisCard::Op => analysis::op(&imp.netlist, Options::analysis()).map(|(r, m)| {
                eprintln!("operating point solved by {m:?}");
                r
            }),
            AnalysisCard::Tran { step, stop } => analysis::transient(
                &imp.netlist,
                stop,
                step,
                Options::analysis(),
                Method::Trapezoidal,
                &progress,
            ),
            AnalysisCard::Dc {
                source,
                start,
                stop,
                step,
            } => {
                let n = ((stop - start) / step).round().abs() as usize;
                let values: Vec<f64> = (0..=n).map(|k| start + step * k as f64).collect();
                let param = if source.to_ascii_uppercase().starts_with('I') {
                    "I"
                } else {
                    "V"
                };
                analysis::dc_sweep(
                    &imp.netlist,
                    &source,
                    param,
                    &values,
                    Options::analysis(),
                    &progress,
                )
            }
            AnalysisCard::Ac {
                per_decade,
                start,
                stop,
            } => analysis::ac(
                &imp.netlist,
                &analysis::log_points(start, stop, per_decade),
                Options::analysis(),
                &progress,
            ),
        }
        .map_err(|e| e.to_string())?;
        print!("{}", result.to_csv());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("electrocraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

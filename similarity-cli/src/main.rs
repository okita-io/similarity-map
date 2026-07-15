mod analyze;
mod input;
mod pass_config;
mod story;

use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use similarity_core::analysis::AnalysisParams;

use crate::analyze::{context_from_story, run_analyze, AnalyzeContext};
use crate::input::JsonAnalysisRequest;
use crate::pass_config::PassConfigFile;
use crate::story::load_chapter;

#[derive(Parser)]
#[command(
    name = "similarity-cli",
    about = "Headless repetition analysis for Romance Factory (AnalysisOutput v1 JSON on stdout)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run embedding repetition analysis and print AnalysisOutput JSON to stdout.
    Analyze(AnalyzeArgs),
}

#[derive(Parser)]
struct AnalyzeArgs {
    /// Romance Factory story directory (loads drafts/chapter_NN.json or chapters/chapter_NN.md).
    #[arg(long, conflicts_with = "input_file")]
    story_path: Option<PathBuf>,

    /// 1-based chapter number (required with --story-path).
    #[arg(long, requires = "story_path")]
    chapter: Option<u32>,

    /// Read `{ text, scope_manifest, params }` JSON from this file instead of stdin.
    #[arg(long, conflicts_with = "story_path")]
    input_file: Option<PathBuf>,

    /// YAML pass bundle (RF `generate:similarity_map:` shape). Runs one or more passes.
    #[arg(long)]
    pass_config: Option<PathBuf>,

    /// Expand edit spans to sentence boundaries before reporting (default: true).
    #[arg(long, default_value_t = true)]
    expand_sentences: bool,

    /// Disable sentence-boundary expansion (overrides --expand-sentences).
    #[arg(long)]
    no_expand_sentences: bool,

    /// Path to all-MiniLM-L6-v2 ONNX model (default: $SIMILARITY_MAP_MODEL_PATH or app data dir).
    #[arg(long)]
    model_path: Option<PathBuf>,

    /// Use deterministic hash embeddings (unit tests / CI without ONNX).
    #[arg(long, hide = true)]
    test_embedder: bool,

    /// Phrase window size when not using --pass-config (default 50).
    #[arg(long, default_value_t = 50)]
    window_size: u32,

    /// Window stride when not using --pass-config (default 10).
    #[arg(long, default_value_t = 10)]
    stride: u32,

    #[arg(long, default_value_t = 2)]
    min_repetitions: u32,

    #[arg(long, default_value_t = 3)]
    min_samples: u32,

    #[arg(long, default_value_t = true)]
    enable_hdbscan: bool,

    #[arg(long, default_value_t = false)]
    link_subphrases: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze(args) => match run_analyze_command(args) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_analyze_command(args: AnalyzeArgs) -> Result<String, String> {
    let expand = args.expand_sentences && !args.no_expand_sentences;
    let pass_config = args
        .pass_config
        .as_ref()
        .map(|p| PassConfigFile::from_yaml_path(p))
        .transpose()?;

    if let Some(ref cfg) = pass_config {
        cfg.validate()?;
    }

    let ctx = if let Some(ref story_path) = args.story_path {
        let chapter = args
            .chapter
            .ok_or_else(|| "--chapter is required with --story-path".to_string())?;
        let draft = load_chapter(story_path, chapter)?;
        let single_params = if pass_config.is_none() {
            Some(default_params(&args))
        } else {
            None
        };
        let expand = pass_config
            .as_ref()
            .map(|c| c.expand_to_sentences)
            .unwrap_or(expand);
        context_from_story(
            draft,
            pass_config,
            single_params,
            expand,
            args.model_path,
            args.test_embedder,
        )
    } else if let Some(ref input_path) = args.input_file {
        let request = JsonAnalysisRequest::from_path(input_path)?;
        build_context_from_request(request, pass_config, &args, expand)?
    } else {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        if buf.trim().is_empty() {
            return Err(
                "no input: provide JSON on stdin, --input-file, or --story-path + --chapter".into(),
            );
        }
        let request = JsonAnalysisRequest::from_reader(buf.as_bytes())?;
        build_context_from_request(request, pass_config, &args, expand)?
    };

    run_analyze(ctx)
}

fn build_context_from_request(
    request: JsonAnalysisRequest,
    pass_config: Option<PassConfigFile>,
    args: &AnalyzeArgs,
    expand: bool,
) -> Result<AnalyzeContext, String> {
    let JsonAnalysisRequest {
        text,
        scope_manifest,
        params,
    } = request;
    let chapter = scope_manifest.chapter;

    if let Some(cfg) = pass_config {
        let expand_to_sentences = cfg.expand_to_sentences;
        Ok(AnalyzeContext {
            text,
            chapter,
            scope_manifest,
            document_path: None,
            document_hash: None,
            pass_config: Some(cfg),
            single_params: None,
            expand_to_sentences,
            model_path: args.model_path.clone(),
            test_embedder: args.test_embedder,
        })
    } else {
        Ok(AnalyzeContext {
            text,
            chapter,
            scope_manifest,
            document_path: None,
            document_hash: None,
            pass_config: None,
            single_params: Some(params.into()),
            expand_to_sentences: expand,
            model_path: args.model_path.clone(),
            test_embedder: args.test_embedder,
        })
    }
}

fn default_params(args: &AnalyzeArgs) -> AnalysisParams {
    AnalysisParams {
        window_size: args.window_size,
        stride: args.stride,
        tokens_per_page: None,
        chapter_break_regex: None,
        min_repetitions: args.min_repetitions,
        min_samples: args.min_samples,
        enable_hdbscan: args.enable_hdbscan,
        link_subphrases: args.link_subphrases,
    }
}

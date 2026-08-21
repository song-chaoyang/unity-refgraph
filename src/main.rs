use clap::Parser;
use unity_refgraph::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        unity_refgraph::cli::Commands::Index { action } => {
            unity_refgraph::cli::run_index_action(action)
        }
        unity_refgraph::cli::Commands::Refs {
            path,
            project,
            direction,
            filter,
        } => unity_refgraph::cli::run_refs(&path, project, &direction, &filter),
        unity_refgraph::cli::Commands::Ls {
            path,
            project,
            depth,
        } => unity_refgraph::cli::run_ls(&path, project, depth),
        unity_refgraph::cli::Commands::Glob {
            pattern,
            project,
            entry_type,
        } => unity_refgraph::cli::run_glob(&pattern, project, &entry_type),
        unity_refgraph::cli::Commands::Grep {
            pattern,
            project,
            path,
        } => unity_refgraph::cli::run_grep(&pattern, project, path.as_deref()),
        unity_refgraph::cli::Commands::Read { path, project } => {
            unity_refgraph::cli::run_read(&path, project)
        }
        unity_refgraph::cli::Commands::Serve { project, port } => {
            unity_refgraph::cli::run_serve(project, port)
        }
        unity_refgraph::cli::Commands::Mcp => unity_refgraph::cli::run_mcp_server(),
    }
}

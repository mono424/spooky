use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType},
};
use inquire::Select;
use inquire::Text;
use std::fs;
use std::io::{self, Write, IsTerminal};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime};



// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

const VERSION: &str = "0.0.1-canary.15";

// ---------------------------------------------------------------------------
// Ghost art (matches AsciiGhost.tsx)
// ---------------------------------------------------------------------------

const RAW_GHOST: [&str; 12] = [
    "   \u{2584}\u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}\u{2584}   ",
    " \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}  \u{2580}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}    \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}  \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} ",
    " \u{2588}\u{2588}\u{2584} \u{2580}\u{2588}\u{2584}\u{2580} \u{2588}\u{2580} \u{2584}\u{2588}\u{2584}\u{2588} ",
    "  \u{2580}   \u{2588}   \u{2588}   \u{2580} \u{2580} ",
];

const EYES_CLOSED: &str = " \u{2588}\u{2588}\u{2588}\u{2588}\u{2580}\u{2580}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}\u{2580}\u{2588}\u{2588}\u{2588}\u{2588} ";

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

mod templates {
    // Schema templates
    pub const SCHEMA_EMPTY: &str = include_str!("templates/schema/empty.surql");
    pub const SCHEMA_MINIMAL: &str = include_str!("templates/schema/minimal.surql");
    pub const SCHEMA_EXAMPLE: &str = include_str!("templates/schema/example.surql");

    // App templates
    pub const APP_PACKAGE_JSON: &str = include_str!("templates/app/package.json.tmpl");
    pub const APP_VITE_CONFIG: &str = include_str!("templates/app/vite.config.ts.tmpl");
    pub const APP_TSCONFIG: &str = include_str!("templates/app/tsconfig.json.tmpl");
    pub const APP_TAILWIND_CONFIG: &str = include_str!("templates/app/tailwind.config.js.tmpl");
    pub const APP_POSTCSS_CONFIG: &str = include_str!("templates/app/postcss.config.js.tmpl");
    pub const APP_INDEX_HTML: &str = include_str!("templates/app/index.html.tmpl");
    pub const APP_MAIN_TSX: &str = include_str!("templates/app/src/main.tsx.tmpl");
    pub const APP_GLOBAL_CSS: &str = include_str!("templates/app/src/global.css.tmpl");
    pub const APP_DB_TS: &str = include_str!("templates/app/src/db.ts.tmpl");
    pub const APP_AUTH_TSX: &str = include_str!("templates/app/src/auth.tsx.tmpl");
    pub const APP_TSX: &str = include_str!("templates/app/src/App.tsx.tmpl");
    pub const APP_NOAUTH_TSX: &str = include_str!("templates/app/src/App.noauth.tsx.tmpl");
    pub const APP_SCHEMA_GEN: &str = include_str!("templates/app/src/schema.gen.ts.tmpl");

    // Shared templates
    pub const GITIGNORE: &str = include_str!("templates/shared/gitignore.tmpl");
    pub const ROOT_PACKAGE_JSON: &str = include_str!("templates/shared/root-package.json.tmpl");
    pub const PNPM_WORKSPACE: &str = include_str!("templates/shared/pnpm-workspace.yaml.tmpl");
    pub const SCHEMA_PACKAGE_JSON: &str = include_str!("templates/shared/schema-package.json.tmpl");
    pub const SPOOKY_YML: &str = include_str!("templates/shared/spooky.yml.tmpl");
}

// ---------------------------------------------------------------------------
// Template rendering
// ---------------------------------------------------------------------------

fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

// ---------------------------------------------------------------------------
// Splash screen animation
// ---------------------------------------------------------------------------

fn shift_line(line: &str, dir: f64) -> String {
    let trimmed = line.trim();
    if dir < -0.7 {
        format!("{}  ", trimmed) // Left
    } else if dir > 0.7 {
        format!("  {}", trimmed) // Right
    } else {
        format!(" {} ", trimmed) // Center
    }
}

fn show_splash() -> Result<()> {
    let mut stdout = io::stdout();

    // Skip animation if not a TTY
    if !stdout.is_terminal() {
        return Ok(());
    }

    let (term_width, term_height) = terminal::size().unwrap_or((80, 24));

    let title = "Spooky";
    let version_line = format!("v{}", VERSION);
    let intro_1 = "Real-time sync for SurrealDB.";
    let intro_2 = "Offline-first. Schema-driven. No backend code.";

    // Total block: 12 ghost + 1 blank + 1 title + 1 version + 1 blank + 2 intro + 1 blank + 1 separator = 20
    let block_height: u16 = 20;
    let start_y = if term_height > block_height + 2 {
        (term_height - block_height - 2) / 2
    } else {
        0
    };

    execute!(stdout, Hide, Clear(ClearType::All))?;

    let ghost_width = 20;
    let nanos_seed = || -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64
    };

    let mut phase: f64 = 0.0;
    let mut blink = false;
    let mut blink_frames = 0u32;

    // Helper to center text
    let center_x = |text_len: usize| -> u16 {
        if (term_width as usize) > text_len {
            ((term_width as usize - text_len) / 2) as u16
        } else {
            0
        }
    };

    for _ in 0..25 {
        phase += 0.5;

        // Random blink: ~5% chance per frame
        if !blink && (nanos_seed() % 20 == 0) {
            blink = true;
            blink_frames = 2;
        }
        if blink {
            if blink_frames == 0 {
                blink = false;
            } else {
                blink_frames -= 1;
            }
        }

        let mut y = start_y;

        // Draw ghost
        for (i, row) in RAW_GHOST.iter().enumerate() {
            let wave_value = ((i as f64) * 0.6 + phase).sin();
            let content = if blink && i == 3 { EYES_CLOSED } else { row };
            let shifted = shift_line(content, wave_value);
            execute!(
                stdout,
                MoveTo(center_x(ghost_width), y),
                SetForegroundColor(Color::White),
                SetAttribute(Attribute::Bold),
                Print(&shifted),
                SetAttribute(Attribute::Reset),
            )?;
            y += 1;
        }

        y += 1; // blank

        // Title
        execute!(
            stdout,
            MoveTo(center_x(title.len()), y),
            SetForegroundColor(Color::White),
            SetAttribute(Attribute::Bold),
            Print(title),
            SetAttribute(Attribute::Reset),
        )?;
        y += 1;

        // Version
        execute!(
            stdout,
            MoveTo(center_x(version_line.len()), y),
            SetForegroundColor(Color::DarkGrey),
            Print(&version_line),
            ResetColor,
        )?;
        y += 2;

        // Intro lines
        execute!(
            stdout,
            MoveTo(center_x(intro_1.len()), y),
            SetForegroundColor(Color::White),
            Print(intro_1),
            ResetColor,
        )?;
        y += 1;

        execute!(
            stdout,
            MoveTo(center_x(intro_2.len()), y),
            SetForegroundColor(Color::White),
            Print(intro_2),
            ResetColor,
        )?;

        stdout.flush()?;
        thread::sleep(Duration::from_millis(100));
    }

    // After animation: move cursor below the content, show it, print separator
    let end_y = start_y + block_height - 1;

    // Draw a dim separator line
    let sep = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
    execute!(
        stdout,
        MoveTo(center_x(40), end_y),
        SetForegroundColor(Color::DarkGrey),
        Print(sep),
        ResetColor,
    )?;

    // Position cursor on the line after the separator for the prompt
    execute!(stdout, MoveTo(0, end_y + 2), Show)?;
    stdout.flush()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn create_project() -> Result<()> {
    show_splash()?;

    // 1. Project name
    let project_name = Text::new("What is the name of your project?").prompt()?;

    // 2. Project kind
    let project_kind = Select::new(
        "What kind of project?",
        vec!["Full project (Schema + App)", "Schema only"],
    )
    .prompt()?;
    let is_schema_only = project_kind == "Schema only";

    // 3. Schema template
    let schema_type = Select::new(
        "Which schema template?",
        vec![
            "Minimal (User + Auth)",
            "Empty",
            "Example (User + Threads + Comments)",
        ],
    )
    .prompt()?;

    let has_auth = schema_type != "Empty";

    // 4. Codegen format
    let codegen_format = if is_schema_only {
        let fmt = Select::new(
            "Which output format for code generation?",
            vec!["TypeScript", "Dart", "Skip"],
        )
        .prompt()?;
        fmt.to_string()
    } else {
        "TypeScript".to_string() // full project always uses TypeScript
    };

    println!(
        "\n  Creating {} project \x1b[1m{}\x1b[0m with {} schema...",
        if is_schema_only { "schema-only" } else { "full" },
        project_name,
        schema_type
    );

    // --- Create root directory ---
    let root_path = Path::new(&project_name);
    if root_path.exists() {
        if !inquire::Confirm::new("Directory already exists. Overwrite?")
            .with_default(false)
            .prompt()?
        {
            println!("Aborting.");
            return Ok(());
        }
        fs::remove_dir_all(root_path)?;
    }
    fs::create_dir_all(root_path)?;

    // --- Determine schema package path ---
    let schema_path = if is_schema_only {
        root_path.to_path_buf()
    } else {
        root_path.join("packages/schema")
    };

    // --- Write schema package ---
    write_schema_package(
        &schema_path,
        &project_name,
        schema_type,
        &codegen_format,
        is_schema_only,
    )?;

    if is_schema_only {
        write_file(root_path.join(".gitignore"), templates::GITIGNORE)?;
    } else {
        write_file(root_path.join(".gitignore"), templates::GITIGNORE)?;

        write_file(
            root_path.join("pnpm-workspace.yaml"),
            templates::PNPM_WORKSPACE,
        )?;

        write_file(
            root_path.join("package.json"),
            &render(templates::ROOT_PACKAGE_JSON, &[
                ("PROJECT_NAME", &project_name),
                ("VERSION", VERSION),
            ]),
        )?;

        write_app_package(root_path, &project_name, has_auth)?;
    }

    println!(
        "\n  \x1b[32m\u{2713}\x1b[0m Project \x1b[1m{}\x1b[0m created",
        project_name
    );

    // --- Optional: git init ---
    let do_git = inquire::Confirm::new("Initialize git repository?")
        .with_default(true)
        .prompt()
        .unwrap_or(false);

    if do_git {
        let git_result = Command::new("git")
            .args(["init"])
            .current_dir(root_path)
            .output();

        match git_result {
            Ok(output) if output.status.success() => {
                // Initial commit
                let _ = Command::new("git")
                    .args(["add", "-A"])
                    .current_dir(root_path)
                    .output();
                let _ = Command::new("git")
                    .args(["commit", "-m", "Initial commit from spooky create"])
                    .current_dir(root_path)
                    .output();
                println!("  \x1b[32m\u{2713}\x1b[0m Git repository initialized");
            }
            _ => {
                println!("  \x1b[33m!\x1b[0m Could not initialize git (is git installed?)");
            }
        }
    }

    // --- Optional: pnpm install ---
    let do_install = inquire::Confirm::new("Install dependencies with pnpm?")
        .with_default(true)
        .prompt()
        .unwrap_or(false);

    if do_install {
        println!("\n  Installing dependencies...");
        let install_result = Command::new("pnpm")
            .args(["install"])
            .current_dir(root_path)
            .status();

        match install_result {
            Ok(status) if status.success() => {
                println!("  \x1b[32m\u{2713}\x1b[0m Dependencies installed");

                // Run spooky generate and migration:create for full projects
                if !is_schema_only {
                    println!("\n  Running spooky generate...");
                    match Command::new("spooky")
                        .args(["generate"])
                        .current_dir(&schema_path)
                        .status()
                    {
                        Ok(s) if s.success() => {
                            println!("  \x1b[32m\u{2713}\x1b[0m Schema types generated");
                        }
                        _ => {
                            println!("  \x1b[33m!\x1b[0m Could not run spooky generate (is spooky installed?)");
                        }
                    }

                    println!("  Creating initial migration...");
                    match Command::new("spooky")
                        .args(["migration:create", "initial"])
                        .current_dir(&schema_path)
                        .status()
                    {
                        Ok(s) if s.success() => {
                            println!("  \x1b[32m\u{2713}\x1b[0m Initial migration created");
                        }
                        _ => {
                            println!("  \x1b[33m!\x1b[0m Could not create initial migration");
                        }
                    }
                }
            }
            _ => {
                println!("  \x1b[33m!\x1b[0m Could not install dependencies (is pnpm installed?)");
            }
        }
    }

    // --- Next steps ---
    println!("\n  \x1b[2mNext steps:\x1b[0m");

    println!("  \x1b[1mcd {}\x1b[0m", project_name);

    if !do_install {
        println!("  \x1b[1mpnpm install\x1b[0m");
        if !is_schema_only {
            println!("  \x1b[1mcd packages/schema && spooky generate\x1b[0m   \x1b[2m# generate schema types\x1b[0m");
            println!("  \x1b[1mspooky migration:create initial\x1b[0m   \x1b[2m# create initial migration\x1b[0m");
        }
    }

    if is_schema_only {
        println!("  \x1b[1mpnpm dev\x1b[0m");
    } else {
        println!(
            "  \x1b[1mcd packages/schema && pnpm dev\x1b[0m   \x1b[2m# start Spooky dev server\x1b[0m"
        );
        println!(
            "  \x1b[1mcd ../../apps/app && pnpm dev\x1b[0m    \x1b[2m# start the app\x1b[0m"
        );
    }

    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema package creation
// ---------------------------------------------------------------------------

fn write_schema_package(
    schema_path: &Path,
    project_name: &str,
    schema_type: &str,
    codegen_format: &str,
    is_schema_only: bool,
) -> Result<()> {
    fs::create_dir_all(schema_path.join("src/buckets"))?;
    fs::create_dir_all(schema_path.join("src/outbox"))?;
    fs::create_dir_all(schema_path.join("migrations"))?;
    fs::create_dir_all(schema_path.join(".spooky"))?;

    // Schema content
    let schema_content = match schema_type {
        "Empty" => templates::SCHEMA_EMPTY,
        "Minimal (User + Auth)" => templates::SCHEMA_MINIMAL,
        "Example (User + Threads + Comments)" => templates::SCHEMA_EXAMPLE,
        _ => templates::SCHEMA_EMPTY,
    };
    write_file(schema_path.join("src/schema.surql"), schema_content)?;

    // Package name
    let pkg_name = if is_schema_only {
        project_name.to_string()
    } else {
        format!("@{}/schema", project_name)
    };

    // Build script (only if codegen is enabled)
    let build_script = if codegen_format == "Skip" {
        ""
    } else {
        ",\n    \"build\": \"spooky generate\""
    };

    write_file(
        schema_path.join("package.json"),
        &render(templates::SCHEMA_PACKAGE_JSON, &[
            ("PACKAGE_NAME", &pkg_name),
            ("BUILD_SCRIPT", build_script),
            ("VERSION", VERSION),
        ]),
    )?;

    // spooky.yml
    let client_types_section = match codegen_format {
        "Skip" => String::new(),
        _ => {
            let fmt = codegen_format.to_lowercase();
            let output = if is_schema_only {
                let ext = match codegen_format {
                    "TypeScript" => "ts",
                    "Dart" => "dart",
                    _ => "ts",
                };
                format!("./schema.gen.{}", ext)
            } else {
                "../../apps/app/src/schema.gen.ts".to_string()
            };
            format!(
                "\nclientTypes:\n  - format: {}\n    output: {}\n    schema: ./src/schema.surql",
                fmt, output
            )
        }
    };

    write_file(
        schema_path.join("spooky.yml"),
        &render(templates::SPOOKY_YML, &[
            ("CLIENT_TYPES_SECTION", &client_types_section),
        ]),
    )?;

    if !is_schema_only {
        write_file(schema_path.join(".gitignore"), templates::GITIGNORE)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// App (SolidJS) package creation
// ---------------------------------------------------------------------------

fn write_app_package(root_path: &Path, project_name: &str, has_auth: bool) -> Result<()> {
    let app_path = root_path.join("apps/app");
    fs::create_dir_all(app_path.join("src"))?;

    let vars: &[(&str, &str)] = &[
        ("PROJECT_NAME", project_name),
        ("VERSION", VERSION),
    ];

    // Config files
    write_file(app_path.join("package.json"), &render(templates::APP_PACKAGE_JSON, vars))?;
    write_file(app_path.join("vite.config.ts"), templates::APP_VITE_CONFIG)?;
    write_file(app_path.join("tsconfig.json"), templates::APP_TSCONFIG)?;
    write_file(app_path.join("tailwind.config.js"), templates::APP_TAILWIND_CONFIG)?;
    write_file(app_path.join("postcss.config.js"), templates::APP_POSTCSS_CONFIG)?;
    write_file(app_path.join("index.html"), &render(templates::APP_INDEX_HTML, vars))?;

    // Source files
    write_file(app_path.join("src/main.tsx"), templates::APP_MAIN_TSX)?;
    write_file(app_path.join("src/global.css"), templates::APP_GLOBAL_CSS)?;
    write_file(app_path.join("src/db.ts"), templates::APP_DB_TS)?;
    write_file(app_path.join("src/schema.gen.ts"), templates::APP_SCHEMA_GEN)?;

    // Auth-conditional files
    if has_auth {
        write_file(app_path.join("src/App.tsx"), &render(templates::APP_TSX, vars))?;
        write_file(app_path.join("src/auth.tsx"), templates::APP_AUTH_TSX)?;
    } else {
        write_file(app_path.join("src/App.tsx"), &render(templates::APP_NOAUTH_TSX, vars))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }
    }
    fs::write(&path, content)
        .with_context(|| format!("Failed to write file {:?}", path.as_ref()))?;
    Ok(())
}

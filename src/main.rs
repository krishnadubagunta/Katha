mod error_codes;

use error_codes::CLI_ERROR;
use katha_parsers::fetch_parser;
use serde_json::to_string_pretty;
use std::env;
use std::path::Path;

fn main() {
    // CLI args that accept file path
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file>", args[0]);
        std::process::exit(CLI_ERROR);
    }

    let file_path = &args[1];

    let file_extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();

    let mut parser = match fetch_parser(file_extension) {
        Ok(parser) => parser,
        Err(error) => {
            eprintln!("Error: {}", error.message());
            std::process::exit(error.code());
        }
    };

    match parser.parse(file_path) {
        Ok(doc) => match to_string_pretty(&doc) {
            Ok(json) => println!("{json}"),
            Err(_) => {
                eprintln!("Error: failed to serialize document");
                std::process::exit(CLI_ERROR);
            }
        },
        Err(error) => {
            eprintln!("Error: {}", error.message());
            std::process::exit(error.code());
        }
    }
}

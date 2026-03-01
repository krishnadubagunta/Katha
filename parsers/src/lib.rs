pub mod docx;
pub mod epub;
pub mod error;
pub mod pdf;

use std::collections::HashMap;

use error::ParserError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub cover_image: String,
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub language: String,
    pub description: String,
    pub toc: Vec<Section>,
    pub content: HashMap<usize, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub content_ref: usize,
    pub children: Vec<Section>,
}

pub trait Parser {
    fn parse(&mut self, src: &str) -> Result<Document, ParserError>;
    fn get_cover(&mut self) -> Result<String, ParserError>;
    fn get_title(&mut self) -> Result<String, ParserError>;
    fn get_subtitle(&mut self) -> Result<String, ParserError>;
    fn get_author(&mut self) -> Result<String, ParserError>;
    fn get_description(&mut self) -> Result<String, ParserError>;
    fn get_publisher(&mut self) -> Result<String, ParserError>;
    fn get_language(&mut self) -> Result<String, ParserError>;
    fn get_cover_image(&mut self) -> Result<String, ParserError>;
    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError>;
    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, String>, ParserError>;
    fn clean_html(html: &str) -> Result<String, ParserError>
    where
        Self: Sized;
}

pub const SUPPORTED_PARSERS: [&str; 3] = ["epub", "docx", "pdf"];

pub fn fetch_parser(parser_type: &str) -> Result<Box<dyn Parser>, ParserError> {
    match parser_type {
        "epub" => Ok(Box::new(epub::Epub::new())),
        "docx" => Ok(Box::new(docx::Docx::new())),
        "pdf" => Ok(Box::new(pdf::Pdf::new())),
        _ => Err(ParserError::UndefinedParser),
    }
}

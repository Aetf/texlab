use std::sync::Arc;

use cstree::TextRange;
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
use multimap::MultiMap;

use crate::{
    syntax::{
        bibtex::{self, HasDelimiters, HasType},
        CstNode,
    },
    Document, LineIndexExt, Uri, Workspace,
};

pub fn analyze_bibtex_static(
    workspace: &dyn Workspace,
    diagnostics_by_uri: &mut MultiMap<Arc<Uri>, Diagnostic>,
    uri: &Uri,
) -> Option<()> {
    let document = workspace.get(uri)?;
    let data = document.data.as_bibtex()?;

    for node in data.root.descendants() {
        analyze_entry(&document, diagnostics_by_uri, node)
            .or_else(|| analyze_field(&document, diagnostics_by_uri, node));
    }

    Some(())
}

fn analyze_entry(
    document: &Document,
    diagnostics_by_uri: &mut MultiMap<Arc<Uri>, Diagnostic>,
    node: &bibtex::SyntaxNode,
) -> Option<()> {
    let entry = bibtex::Entry::cast(node)?;
    if entry.left_delimiter().is_none() {
        diagnostics_by_uri.insert(
            Arc::clone(&document.uri),
            Diagnostic {
                range: document
                    .line_index
                    .line_col_lsp_range(entry.ty()?.text_range()),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::Number(4)),
                code_description: None,
                source: Some("texlab".to_string()),
                message: "Expecting a curly bracket: \"{\"".to_string(),
                related_information: None,
                tags: None,
                data: None,
            },
        );
        return Some(());
    }

    if entry.key().is_none() {
        diagnostics_by_uri.insert(
            Arc::clone(&document.uri),
            Diagnostic {
                range: document
                    .line_index
                    .line_col_lsp_range(entry.left_delimiter()?.text_range()),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::Number(5)),
                code_description: None,
                source: Some("texlab".to_string()),
                message: "Expecting a key".to_string(),
                related_information: None,
                tags: None,
                data: None,
            },
        );
        return Some(());
    }

    if entry.key().is_none() {
        diagnostics_by_uri.insert(
            Arc::clone(&document.uri),
            Diagnostic {
                range: document
                    .line_index
                    .line_col_lsp_range(entry.right_delimiter()?.text_range()),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::Number(6)),
                code_description: None,
                source: Some("texlab".to_string()),
                message: "Expecting a curly bracket: \"}\"".to_string(),
                related_information: None,
                tags: None,
                data: None,
            },
        );
        return Some(());
    }

    Some(())
}

fn analyze_field(
    document: &Document,
    diagnostics_by_uri: &mut MultiMap<Arc<Uri>, Diagnostic>,
    node: &bibtex::SyntaxNode,
) -> Option<()> {
    let field = bibtex::Field::cast(node)?;
    if field.equality_sign().is_none() {
        diagnostics_by_uri.insert(
            Arc::clone(&document.uri),
            Diagnostic {
                range: document
                    .line_index
                    .line_col_lsp_range(TextRange::empty(field.name()?.text_range().end())),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::Number(7)),
                code_description: None,
                source: Some("texlab".to_string()),
                message: "Expecting an equality sign: \"=\"".to_string(),
                related_information: None,
                tags: None,
                data: None,
            },
        );
        return Some(());
    }

    if field.value().is_none() {
        diagnostics_by_uri.insert(
            Arc::clone(&document.uri),
            Diagnostic {
                range: document.line_index.line_col_lsp_range(TextRange::empty(
                    field.equality_sign()?.text_range().end(),
                )),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::Number(8)),
                code_description: None,
                source: Some("texlab".to_string()),
                message: "Expecting a field value".to_string(),
                related_information: None,
                tags: None,
                data: None,
            },
        );
        return Some(());
    }

    Some(())
}

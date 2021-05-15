use std::collections::HashMap;

use cancellation::CancellationToken;
use lsp_types::{Range, RenameParams, TextEdit, WorkspaceEdit};

use crate::{
    features::cursor::{CursorContext, HasPosition},
    syntax::{latex, CstNode},
    LineIndexExt,
};

pub fn prepare_label_rename<P: HasPosition>(
    context: &CursorContext<P>,
    _cancellation_token: &CancellationToken,
) -> Option<Range> {
    let name = context.cursor.as_latex()?;
    name.parent().parent().filter(|node| {
        matches!(
            node.kind(),
            latex::LABEL_DEFINITION | latex::LABEL_REFERENCE | latex::LABEL_REFERENCE_RANGE
        )
    })?;

    Some(
        context
            .request
            .main_document()
            .line_index
            .line_col_lsp_range(name.text_range()),
    )
}

pub fn rename_label(
    context: &CursorContext<RenameParams>,
    cancellation_token: &CancellationToken,
) -> Option<WorkspaceEdit> {
    prepare_label_rename(context, cancellation_token)?;
    let name_text = context.cursor.as_latex()?.text();
    let mut changes = HashMap::new();
    for document in &context.request.subset.documents {
        cancellation_token.result().ok()?;
        if let Some(data) = document.data.as_latex() {
            let mut edits = Vec::new();
            for node in data.root.descendants() {
                if let Some(range) = latex::LabelDefinition::cast(node)
                    .and_then(|label| label.name())
                    .and_then(|name| name.word())
                    .filter(|name| name.text() == name_text)
                    .map(|name| document.line_index.line_col_lsp_range(name.text_range()))
                {
                    edits.push(TextEdit::new(
                        range,
                        context.request.params.new_name.clone(),
                    ));
                }

                latex::LabelReference::cast(node)
                    .and_then(|label| label.name_list())
                    .into_iter()
                    .flat_map(|label| label.words())
                    .filter(|name| name.text() == name_text)
                    .map(|name| document.line_index.line_col_lsp_range(name.text_range()))
                    .for_each(|range| {
                        edits.push(TextEdit::new(
                            range,
                            context.request.params.new_name.clone(),
                        ));
                    });

                if let Some(label) = latex::LabelReferenceRange::cast(node) {
                    if let Some(name1) = label
                        .from()
                        .and_then(|name| name.word())
                        .filter(|name| name.text() == name_text)
                    {
                        edits.push(TextEdit::new(
                            document.line_index.line_col_lsp_range(name1.text_range()),
                            context.request.params.new_name.clone(),
                        ));
                    }

                    if let Some(name2) = label
                        .from()
                        .and_then(|name| name.word())
                        .filter(|name| name.text() == name_text)
                    {
                        edits.push(TextEdit::new(
                            document.line_index.line_col_lsp_range(name2.text_range()),
                            context.request.params.new_name.clone(),
                        ));
                    }
                }
            }

            changes.insert(document.uri.as_ref().clone().into(), edits);
        }
    }

    Some(WorkspaceEdit::new(changes))
}

#[cfg(test)]
mod tests {
    use crate::{features::testing::FeatureTester, RangeExt};

    use super::*;

    #[test]
    fn test_label() {
        let tester = FeatureTester::builder()
            .files(vec![
                ("foo.tex", r#"\label{foo}\include{bar}"#),
                ("bar.tex", r#"\ref{foo}"#),
                ("baz.tex", r#"\ref{foo}"#),
            ])
            .main("foo.tex")
            .line(0)
            .character(7)
            .new_name("bar")
            .build();

        let uri1 = tester.uri("foo.tex");
        let uri2 = tester.uri("bar.tex");
        let request = tester.rename();

        let context = CursorContext::new(request);
        let actual_edit = rename_label(&context, CancellationToken::none()).unwrap();

        let mut expected_changes = HashMap::new();
        expected_changes.insert(
            uri1.as_ref().clone().into(),
            vec![TextEdit::new(Range::new_simple(0, 7, 0, 10), "bar".into())],
        );
        expected_changes.insert(
            uri2.as_ref().clone().into(),
            vec![TextEdit::new(Range::new_simple(0, 5, 0, 8), "bar".into())],
        );
        let expected_edit = WorkspaceEdit::new(expected_changes);

        assert_eq!(actual_edit, expected_edit);
    }
}

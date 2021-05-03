use cancellation::CancellationToken;
use cstree::TextRange;
use lsp_types::CompletionParams;

use crate::{
    features::cursor::CursorContext,
    syntax::{latex, CstNode},
};

use super::types::{InternalCompletionItem, InternalCompletionItemData};

pub fn complete_glossary_entries<'a>(
    context: &'a CursorContext<CompletionParams>,
    items: &mut Vec<InternalCompletionItem<'a>>,
    cancellation_token: &CancellationToken,
) -> Option<()> {
    cancellation_token.result().ok()?;

    let token = context.cursor.as_latex()?;
    let group = latex::CurlyGroupWord::cast(token.parent())
        .filter(|group| context.is_inside_latex_curly(group))?;
    latex::GlossaryEntryReference::cast(group.syntax().parent()?)?;
    let range = if token.kind() == latex::WORD {
        token.text_range()
    } else {
        TextRange::empty(context.offset)
    };

    for document in &context.request.subset.documents {
        if let Some(data) = document.data.as_latex() {
            for node in data.root.descendants() {
                cancellation_token.result().ok()?;

                if let Some(name) = latex::GlossaryEntryDefinition::cast(node)
                    .and_then(|entry| entry.name())
                    .and_then(|name| name.word())
                    .map(|name| name.text())
                {
                    items.push(InternalCompletionItem::new(
                        range,
                        InternalCompletionItemData::GlossaryEntry { name },
                    ));
                } else if let Some(name) = latex::AcroynmDefinition::cast(node)
                    .and_then(|entry| entry.name())
                    .and_then(|name| name.word())
                    .map(|name| name.text())
                {
                    items.push(InternalCompletionItem::new(
                        range,
                        InternalCompletionItemData::Acronym { name },
                    ));
                }
            }
        }
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use cstree::TextRange;

    use crate::features::testing::FeatureTester;

    use super::*;

    #[test]
    fn test_empty_latex_document() {
        let request = FeatureTester::builder()
            .files(vec![("main.tex", "")])
            .main("main.tex")
            .line(0)
            .character(0)
            .build()
            .completion();

        let context = CursorContext::new(request);
        let mut actual_items = Vec::new();
        complete_glossary_entries(&context, &mut actual_items, CancellationToken::none());

        assert!(actual_items.is_empty());
    }

    #[test]
    fn test_empty_bibtex_document() {
        let request = FeatureTester::builder()
            .files(vec![("main.bib", "")])
            .main("main.bib")
            .line(0)
            .character(0)
            .build()
            .completion();

        let context = CursorContext::new(request);
        let mut actual_items = Vec::new();
        complete_glossary_entries(&context, &mut actual_items, CancellationToken::none());

        assert!(actual_items.is_empty());
    }

    #[test]
    fn test_simple() {
        let request = FeatureTester::builder()
            .files(vec![("main.tex", "\\newacronym[longplural={Frames per Second}]{fpsLabel}{FPS}{Frame per Second}\n\\gls{f}")])
            .main("main.tex")
            .line(1)
            .character(6)
            .build()
            .completion();

        let context = CursorContext::new(request);
        let mut actual_items = Vec::new();
        complete_glossary_entries(&context, &mut actual_items, CancellationToken::none());

        assert!(!actual_items.is_empty());
        for item in actual_items {
            assert_eq!(item.range, TextRange::new(82.into(), 83.into()));
        }
    }

    #[test]
    fn test_open_brace() {
        let request = FeatureTester::builder()
        .files(vec![("main.tex", "\\newacronym[longplural={Frames per Second}]{fpsLabel}{FPS}{Frame per Second}\n\\gls{f")])
        .main("main.tex")
        .line(1)
        .character(6)
        .build()
        .completion();

        let context = CursorContext::new(request);
        let mut actual_items = Vec::new();
        complete_glossary_entries(&context, &mut actual_items, CancellationToken::none());

        assert!(!actual_items.is_empty());
        for item in actual_items {
            assert_eq!(item.range, TextRange::new(82.into(), 83.into()));
        }
    }
}

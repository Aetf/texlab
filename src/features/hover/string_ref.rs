use cancellation::CancellationToken;
use lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind};

use crate::{
    features::FeatureRequest,
    syntax::{bibtex, CstNode},
    LineIndexExt,
};

pub fn find_string_reference_hover(
    request: &FeatureRequest<HoverParams>,
    token: &CancellationToken,
) -> Option<Hover> {
    let main_document = request.main_document();
    let data = main_document.data.as_bibtex()?;
    let offset = main_document
        .line_index
        .offset_lsp(request.params.text_document_position_params.position);

    let name = data
        .root
        .token_at_offset(offset)
        .right_biased()
        .filter(|token| token.kind() == bibtex::WORD)?;

    if !matches!(name.parent().kind(), bibtex::TOKEN | bibtex::STRING) {
        return None;
    }

    for string in data.root.children().filter_map(bibtex::String::cast) {
        if token.is_canceled() {
            return None;
        }

        if string.name().filter(|n| n.text() == name.text()).is_some() {
            let value = string.value()?.syntax().text().to_string();
            return Some(Hover {
                range: Some(
                    main_document
                        .line_index
                        .line_col_lsp_range(name.text_range()),
                ),
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::PlainText,
                    value,
                }),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use lsp_types::Range;

    use crate::{features::testing::FeatureTester, RangeExt};

    use super::*;

    #[test]
    fn test_empty_latex_document() {
        let request = FeatureTester::builder()
            .files(vec![("main.tex", "")])
            .main("main.tex")
            .line(0)
            .character(0)
            .build()
            .hover();

        let actual_hover = find_string_reference_hover(&request, CancellationToken::none());

        assert_eq!(actual_hover, None);
    }

    #[test]
    fn test_empty_bibtex_document() {
        let request = FeatureTester::builder()
            .files(vec![("main.bib", "")])
            .main("main.bib")
            .line(0)
            .character(0)
            .build()
            .hover();

        let actual_hover = find_string_reference_hover(&request, CancellationToken::none());

        assert_eq!(actual_hover, None);
    }

    #[test]
    fn test_inside_reference() {
        let request = FeatureTester::builder()
            .files(vec![(
                "main.bib",
                indoc! { r#"
                    @string{foo = "Foo"}
                    @string{bar = "Bar"}
                    @article{baz, author = bar}
                "# },
            )])
            .main("main.bib")
            .line(2)
            .character(24)
            .build()
            .hover();

        let actual_hover =
            find_string_reference_hover(&request, CancellationToken::none()).unwrap();

        let expected_hover = Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: "\"Bar\"".into(),
            }),
            range: Some(Range::new_simple(2, 23, 2, 26)),
        };

        assert_eq!(actual_hover, expected_hover);
    }

    #[test]
    fn test_inside_field() {
        let request = FeatureTester::builder()
            .files(vec![(
                "main.bib",
                indoc! { r#"
                    @string{foo = "Foo"}
                    @string{bar = "Bar"}
                    @article{baz, author = bar}
                "# },
            )])
            .main("main.bib")
            .line(2)
            .character(20)
            .build()
            .hover();

        let actual_hover = find_string_reference_hover(&request, CancellationToken::none());
        assert_eq!(actual_hover, None);
    }
}

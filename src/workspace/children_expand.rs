use std::sync::Arc;

use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::{
    Document, DocumentLanguage, OpenHandler, Uri, Workspace, WorkspaceSource, WorkspaceSubset,
};

pub struct ChildrenExpander<W> {
    workspace: Arc<W>,
}

impl<W> ChildrenExpander<W>
where
    W: Workspace + Send + Sync + 'static,
{
    pub fn new(workspace: Arc<W>) -> Self {
        let ws = Arc::clone(&workspace);
        workspace.register_open_handler(Arc::new(move |document| {
            Self::expand(&ws, &document);
        }));
        Self { workspace }
    }

    fn expand(workspace: &W, document: &Document) {
        if let Some(data) = document.data.as_latex() {
            let extras = &data.extras;
            let mut all_targets = vec![&extras.implicit_links.aux, &extras.implicit_links.log];
            for link in &extras.explicit_links {
                all_targets.push(&link.targets);
            }

            all_targets.into_par_iter().for_each(|targets| {
                for path in targets
                    .into_iter()
                    .filter(|uri| uri.scheme() == "file" && uri.fragment().is_none())
                    .filter_map(|uri| uri.to_file_path().ok())
                {
                    if workspace.load(path).is_ok() {
                        break;
                    }
                }
            });
        }
    }
}

impl<W: Workspace> Workspace for ChildrenExpander<W> {
    fn open(
        &self,
        uri: Arc<Uri>,
        text: String,
        language: DocumentLanguage,
        source: WorkspaceSource,
    ) -> Arc<Document> {
        self.workspace.open(uri, text, language, source)
    }

    fn register_open_handler(&self, handler: OpenHandler) {
        self.workspace.register_open_handler(handler)
    }

    fn documents(&self) -> Vec<Arc<Document>> {
        self.workspace.documents()
    }

    fn has(&self, uri: &Uri) -> bool {
        self.workspace.has(uri)
    }

    fn get(&self, uri: &Uri) -> Option<Arc<Document>> {
        self.workspace.get(uri)
    }

    fn close(&self, uri: &Uri) {
        self.workspace.close(uri)
    }

    fn is_open(&self, uri: &Uri) -> bool {
        self.workspace.is_open(uri)
    }

    fn subset(&self, uri: Arc<Uri>) -> Option<WorkspaceSubset> {
        self.workspace.subset(uri)
    }
}

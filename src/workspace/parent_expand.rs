use std::{fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use notify::RecursiveMode;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rustc_hash::FxHashSet;

use crate::{
    Document, DocumentLanguage, OpenHandler, Uri, Workspace, WorkspaceSource, WorkspaceSubset,
};

pub struct ParentExpander<W> {
    workspace: W,
}

impl<W: Workspace> ParentExpander<W> {
    pub fn new(workspace: W) -> Self {
        Self { workspace }
    }
}

impl<W> Workspace for ParentExpander<W>
where
    W: Workspace + Send + Sync + 'static,
{
    fn open(
        &self,
        uri: Arc<Uri>,
        text: String,
        language: DocumentLanguage,
        source: WorkspaceSource,
    ) -> Arc<Document> {
        let document = self
            .workspace
            .open(Arc::clone(&uri), text, language, source);

        let all_current_paths = self
            .workspace
            .documents()
            .into_iter()
            .filter_map(|doc| doc.uri.to_file_path().ok())
            .collect::<FxHashSet<_>>();

        if uri.scheme() == "file" {
            if let Ok(mut path) = uri.to_file_path() {
                while path.pop() && !self.has_parent(Arc::clone(&uri)).unwrap_or(false) {
                    let mut files = Vec::new();
                    fs::read_dir(&path)
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| entry.file_type().ok().filter(|ty| ty.is_file()).is_some())
                        .map(|entry| entry.path())
                        .filter(|path| {
                            matches!(
                                DocumentLanguage::by_path(&path),
                                Some(DocumentLanguage::Latex)
                            )
                        })
                        .filter(|path| !all_current_paths.contains(path))
                        .for_each(|path| {
                            files.push(path);
                        });
                    files.into_par_iter().for_each(|path| {
                        let _ = self.workspace.load(path);
                    });
                }
            }
        }

        document
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

    fn watch(&self, path: PathBuf, mode: RecursiveMode) -> Result<()> {
        self.workspace.watch(path, mode)
    }
}

impl<W> ParentExpander<W>
where
    W: Workspace + Send + Sync + 'static,
{
    fn has_parent(&self, uri: Arc<Uri>) -> Option<bool> {
        let subset = self.subset(Arc::clone(&uri))?;
        Some(subset.documents.iter().any(|document| {
            document
                .data
                .as_latex()
                .map(|data| {
                    data.extras.has_document_environment
                        && !data
                            .extras
                            .explicit_links
                            .iter()
                            .filter_map(|link| link.as_component_name())
                            .any(|name| name == "subfiles.cls")
                })
                .unwrap_or(false)
        }))
    }
}

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::Result;
use cancellation::{CancellationToken, CancellationTokenSource};
use crossbeam_channel::Sender;
use log::{info, warn};
use lsp_server::{Connection, ErrorCode, Message, RequestId};
use lsp_types::{
    notification::{
        Cancel, DidChangeConfiguration, DidChangeTextDocument, DidOpenTextDocument,
        DidSaveTextDocument, LogMessage, PublishDiagnostics,
    },
    request::{
        DocumentLinkRequest, FoldingRangeRequest, Formatting, GotoDefinition, PrepareRenameRequest,
        References, Rename, SemanticTokensRangeRequest,
    },
    *,
};
use notification::DidCloseTextDocument;
use request::{
    Completion, DocumentHighlightRequest, DocumentSymbolRequest, HoverRequest,
    ResolveCompletionItem, WorkspaceSymbol,
};
use serde::Serialize;
use threadpool::ThreadPool;

use crate::{
    client::send_notification,
    component_db::COMPONENT_DATABASE,
    config::{pull_config, push_config, register_config_capability},
    create_workspace_full,
    diagnostics::{DiagnosticsDebouncer, DiagnosticsManager, DiagnosticsMessage},
    dispatch::{NotificationDispatcher, RequestDispatcher},
    distro::Distribution,
    features::{
        build_document, find_all_references, find_document_highlights, find_document_links,
        find_document_symbols, find_foldings, find_hover, find_workspace_symbols,
        format_source_code, goto_definition, prepare_rename_all, rename_all, BuildParams,
        BuildResult, FeatureRequest, ForwardSearchResult,
    },
    req_queue::{IncomingData, ReqQueue},
    DocumentLanguage, ServerContext, Uri, Workspace, WorkspaceSource,
};

pub struct Server {
    connection: Connection,
    context: Arc<ServerContext>,
    req_queue: Arc<Mutex<ReqQueue>>,
    workspace: Arc<dyn Workspace>,
    static_debouncer: DiagnosticsDebouncer,
    chktex_debouncer: DiagnosticsDebouncer,
    pool: ThreadPool,
    load_resolver: bool,
}

impl Server {
    pub fn with_connection(
        connection: Connection,
        current_dir: PathBuf,
        load_resolver: bool,
    ) -> Result<Self> {
        let context = Arc::new(ServerContext::new(current_dir));
        let req_queue = Arc::default();
        let workspace = Arc::new(create_workspace_full(Arc::clone(&context))?);
        let diag_manager = Arc::new(Mutex::new(DiagnosticsManager::default()));

        let static_debouncer =
            create_static_debouncer(Arc::clone(&diag_manager), &connection, Arc::clone(&context));

        let chktex_debouncer =
            create_chktex_debouncer(diag_manager, &connection, Arc::clone(&context));

        Ok(Self {
            connection,
            context,
            req_queue,
            workspace,
            static_debouncer,
            chktex_debouncer,
            pool: threadpool::Builder::new().build(),
            load_resolver,
        })
    }

    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::Full),
                    will_save: None,
                    will_save_wait_until: None,
                    save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                        include_text: Some(false),
                    })),
                },
            )),
            document_link_provider: Some(DocumentLinkOptions {
                resolve_provider: Some(false),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            }),
            folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            #[cfg(feature = "completion")]
            completion_provider: Some(CompletionOptions {
                resolve_provider: Some(true),
                trigger_characters: Some(vec![
                    "\\".into(),
                    "{".into(),
                    "}".into(),
                    "@".into(),
                    "/".into(),
                    " ".into(),
                ]),
                ..CompletionOptions::default()
            }),
            document_symbol_provider: Some(OneOf::Left(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            rename_provider: Some(OneOf::Right(RenameOptions {
                prepare_provider: Some(true),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            })),
            document_highlight_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            #[cfg(feature = "semantic")]
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    full: None,
                    range: Some(true),
                    legend: SemanticTokensLegend {
                        token_types: crate::features::legend::SUPPORTED_TYPES.to_vec(),
                        token_modifiers: crate::features::legend::SUPPORTED_MODIFIERS.to_vec(),
                    },
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
            ),
            ..ServerCapabilities::default()
        }
    }

    fn initialize(&mut self) -> Result<()> {
        let (id, params) = self.connection.initialize_start()?;
        let params: InitializeParams = serde_json::from_value(params)?;

        *self.context.client_capabilities.lock().unwrap() = params.capabilities;
        *self.context.client_info.lock().unwrap() = params.client_info;

        let result = InitializeResult {
            capabilities: self.capabilities(),
            server_info: Some(ServerInfo {
                name: "TexLab".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        };
        self.connection
            .initialize_finish(id, serde_json::to_value(result)?)?;

        let cx = Arc::clone(&self.context);
        if self.load_resolver {
            self.pool.execute(move || {
                let distro = Distribution::detect();
                info!("Detected distribution: {}", distro.kind);
                *cx.resolver.lock().unwrap() = distro.resolver;
            });
        }

        self.register_diagnostics_handler();

        let req_queue = Arc::clone(&self.req_queue);
        let sender = self.connection.sender.clone();
        let context = Arc::clone(&self.context);
        self.pool.execute(move || {
            register_config_capability(&req_queue, &sender, &context.client_capabilities);
            pull_config(
                &req_queue,
                &sender,
                &context.options,
                &context.client_capabilities.lock().unwrap(),
            );
        });
        Ok(())
    }

    fn register_diagnostics_handler(&mut self) {
        let sender = self.static_debouncer.sender.clone();
        self.workspace
            .register_open_handler(Arc::new(move |workspace, document| {
                let message = DiagnosticsMessage::Analyze {
                    workspace,
                    document,
                };
                sender.send(message).unwrap();
            }));
    }

    fn register_incoming_request(&self, id: RequestId) -> Arc<CancellationToken> {
        let token_source = CancellationTokenSource::new();
        let token = Arc::clone(token_source.token());
        let mut req_queue = self.req_queue.lock().unwrap();
        req_queue
            .incoming
            .register(id.clone(), IncomingData { token_source });
        token
    }

    fn cancel(&self, params: CancelParams) -> Result<()> {
        let id = match params.id {
            NumberOrString::Number(id) => RequestId::from(id),
            NumberOrString::String(id) => RequestId::from(id),
        };

        let mut req_queue = self.req_queue.lock().unwrap();
        if let Some(data) = req_queue.incoming.complete(id.clone()) {
            data.token_source.cancel();
        }

        Ok(())
    }

    fn did_change_configuration(&self, params: DidChangeConfigurationParams) -> Result<()> {
        push_config(&self.context.options, params.settings);
        Ok(())
    }

    fn did_open(&self, params: DidOpenTextDocumentParams) -> Result<()> {
        let language_id = &params.text_document.language_id;
        let language = DocumentLanguage::by_language_id(language_id);
        let document = self.workspace.open(
            Arc::new(params.text_document.uri.into()),
            params.text_document.text,
            language.unwrap_or(DocumentLanguage::Latex),
            WorkspaceSource::Client,
        );

        let should_lint = { self.context.options.read().unwrap().chktex.on_open_and_save };
        if let Some(document) = self
            .workspace
            .get(document.uri.as_ref())
            .filter(|_| should_lint)
        {
            self.chktex_debouncer
                .sender
                .send(DiagnosticsMessage::Analyze {
                    workspace: Arc::clone(&self.workspace),
                    document,
                })?;
        };
        Ok(())
    }

    fn did_change(&self, mut params: DidChangeTextDocumentParams) -> Result<()> {
        let uri = params.text_document.uri.into();
        assert_eq!(params.content_changes.len(), 1);
        let text = params.content_changes.pop().unwrap().text;
        let language = self
            .workspace
            .get(&uri)
            .map(|document| document.data.language())
            .unwrap_or(DocumentLanguage::Latex);

        let document = self
            .workspace
            .open(Arc::new(uri), text, language, WorkspaceSource::Client);

        let should_lint = { self.context.options.read().unwrap().chktex.on_edit };
        if let Some(document) = self
            .workspace
            .get(document.uri.as_ref())
            .filter(|_| should_lint)
        {
            self.chktex_debouncer
                .sender
                .send(DiagnosticsMessage::Analyze {
                    workspace: Arc::clone(&self.workspace),
                    document,
                })?;
        };

        Ok(())
    }

    fn did_save(&self, params: DidSaveTextDocumentParams) -> Result<()> {
        let uri = params.text_document.uri.into();
        let should_lint = { self.context.options.read().unwrap().chktex.on_open_and_save };
        if let Some(document) = self.workspace.get(&uri).filter(|_| should_lint) {
            self.chktex_debouncer
                .sender
                .send(DiagnosticsMessage::Analyze {
                    workspace: Arc::clone(&self.workspace),
                    document,
                })?;
        };
        Ok(())
    }

    fn did_close(&self, params: DidCloseTextDocumentParams) -> Result<()> {
        let uri = params.text_document.uri.into();
        self.workspace.close(&uri);
        Ok(())
    }

    fn feature_request<P>(&self, uri: Arc<Uri>, params: P) -> Option<FeatureRequest<P>> {
        let req_queue = Arc::clone(&self.req_queue);
        let sender = self.connection.sender.clone();
        let cx = Arc::clone(&self.context);
        self.pool.execute(move || {
            pull_config(
                &req_queue,
                &sender,
                &cx.options,
                &cx.client_capabilities.lock().unwrap(),
            );
        });

        Some(FeatureRequest {
            context: Arc::clone(&self.context),
            params,
            workspace: Arc::clone(&self.workspace),
            subset: self.workspace.subset(uri)?,
        })
    }

    fn send_feature_error(&self, id: RequestId) -> Result<()> {
        let resp = lsp_server::Response::new_err(
            id,
            ErrorCode::InternalError as i32,
            "unknown document URI".to_string(),
        );
        self.connection.sender.send(resp.into())?;
        Ok(())
    }

    fn handle_feature_request<P, R, H>(
        &self,
        id: RequestId,
        params: P,
        uri: Arc<Uri>,
        token: &Arc<CancellationToken>,
        handler: H,
    ) -> Result<()>
    where
        P: Send + 'static,
        R: Serialize,
        H: FnOnce(FeatureRequest<P>, &CancellationToken) -> R + Send + 'static,
    {
        match self.feature_request(uri, params) {
            Some(req) => {
                let sender = self.connection.sender.clone();
                let token = Arc::clone(token);
                self.pool.execute(move || {
                    let result = handler(req, &token);
                    if token.is_canceled() {
                        sender.send(cancel_response(id).into()).unwrap();
                    } else {
                        sender
                            .send(lsp_server::Response::new_ok(id, result).into())
                            .unwrap();
                    }
                });
            }
            None => {
                self.send_feature_error(id)?;
            }
        };
        Ok(())
    }

    fn document_link(
        &self,
        id: RequestId,
        params: DocumentLinkParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(id, params, uri, token, find_document_links)?;
        Ok(())
    }

    fn document_symbols(
        &self,
        id: RequestId,
        params: DocumentSymbolParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(id, params, uri, token, find_document_symbols)?;
        Ok(())
    }

    fn workspace_symbols(
        &self,
        id: RequestId,
        params: WorkspaceSymbolParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let sender = self.connection.sender.clone();
        let workspace = Arc::clone(&self.workspace);
        let token = Arc::clone(token);
        self.pool.execute(move || {
            let result = find_workspace_symbols(workspace.as_ref(), &params, &token);
            if token.is_canceled() {
                sender.send(cancel_response(id).into()).unwrap();
            } else {
                sender
                    .send(lsp_server::Response::new_ok(id, result).into())
                    .unwrap();
            }
        });
        Ok(())
    }

    #[cfg(feature = "completion")]
    fn completion(
        &self,
        id: RequestId,
        params: CompletionParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, crate::features::complete)?;
        Ok(())
    }

    #[cfg(feature = "completion")]
    fn completion_resolve(
        &self,
        id: RequestId,
        mut item: CompletionItem,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let sender = self.connection.sender.clone();
        let token = Arc::clone(token);
        let workspace = Arc::clone(&self.workspace);
        self.pool.execute(move || {
            match serde_json::from_value(item.data.clone().unwrap()).unwrap() {
                crate::features::CompletionItemData::Package
                | crate::features::CompletionItemData::Class => {
                    item.documentation = COMPONENT_DATABASE
                        .documentation(&item.label)
                        .map(Documentation::MarkupContent);
                }
                #[cfg(feature = "citeproc")]
                crate::features::CompletionItemData::Citation { uri, key } => {
                    if let Some(document) = workspace.get(&uri) {
                        if let Some(data) = document.data.as_bibtex() {
                            let markup = crate::citation::render_citation(&data.root, &key);
                            item.documentation = markup.map(Documentation::MarkupContent);
                        }
                    }
                }
                _ => {}
            };

            drop(workspace);
            if token.is_canceled() {
                sender.send(cancel_response(id).into()).unwrap();
            } else {
                sender
                    .send(lsp_server::Response::new_ok(id, item).into())
                    .unwrap();
            }
        });
        Ok(())
    }

    fn folding_range(
        &self,
        id: RequestId,
        params: FoldingRangeParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(id, params, uri, token, find_foldings)?;
        Ok(())
    }

    fn references(
        &self,
        id: RequestId,
        params: ReferenceParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, find_all_references)?;
        Ok(())
    }

    fn hover(
        &self,
        id: RequestId,
        params: HoverParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position_params
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, find_hover)?;
        Ok(())
    }

    fn goto_definition(
        &self,
        id: RequestId,
        params: GotoDefinitionParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position_params
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, goto_definition)?;
        Ok(())
    }

    fn prepare_rename(
        &self,
        id: RequestId,
        params: TextDocumentPositionParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(id, params, uri, token, prepare_rename_all)?;
        Ok(())
    }

    fn rename(
        &self,
        id: RequestId,
        params: RenameParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, rename_all)?;
        Ok(())
    }

    fn document_highlight(
        &self,
        id: RequestId,
        params: DocumentHighlightParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(
            params
                .text_document_position_params
                .text_document
                .uri
                .clone()
                .into(),
        );
        self.handle_feature_request(id, params, uri, token, find_document_highlights)?;
        Ok(())
    }

    fn formatting(
        &self,
        id: RequestId,
        params: DocumentFormattingParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(id, params, uri, token, format_source_code)?;
        Ok(())
    }

    #[cfg(feature = "semantic")]
    fn semantic_tokens_range(
        &self,
        id: RequestId,
        params: SemanticTokensRangeParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(
            id,
            params,
            uri,
            token,
            crate::features::find_semantic_tokens_range,
        )?;
        Ok(())
    }

    #[cfg(not(feature = "semantic"))]
    fn semantic_tokens_range(
        &self,
        _id: RequestId,
        _params: SemanticTokensRangeParams,
        _token: &Arc<CancellationToken>,
    ) -> Result<()> {
        Ok(())
    }

    fn build(
        &self,
        id: RequestId,
        params: BuildParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        let lsp_sender = self.connection.sender.clone();
        self.handle_feature_request(id, params, uri, token, |request, token| {
            let (log_sender, log_receiver) = crossbeam_channel::unbounded();

            thread::spawn(move || {
                for message in &log_receiver {
                    send_notification::<LogMessage>(
                        &lsp_sender,
                        LogMessageParams {
                            message,
                            typ: MessageType::Log,
                        },
                    )
                    .unwrap();
                }
            });

            build_document(request, token, log_sender)
        })?;
        Ok(())
    }

    fn forward_search(
        &self,
        id: RequestId,
        params: TextDocumentPositionParams,
        token: &Arc<CancellationToken>,
    ) -> Result<()> {
        let uri = Arc::new(params.text_document.uri.clone().into());
        self.handle_feature_request(
            id,
            params,
            uri,
            token,
            crate::features::execute_forward_search,
        )?;
        Ok(())
    }

    fn process_messages(&self) -> Result<()> {
        for msg in &self.connection.receiver {
            match msg {
                Message::Request(request) => {
                    if self.connection.handle_shutdown(&request)? {
                        return Ok(());
                    }

                    let token = self.register_incoming_request(request.id.clone());
                    if let Some(response) = RequestDispatcher::new(request)
                        .on::<DocumentLinkRequest, _>(|id, params| {
                            self.document_link(id, params, &token)
                        })?
                        .on::<FoldingRangeRequest, _>(|id, params| {
                            self.folding_range(id, params, &token)
                        })?
                        .on::<References, _>(|id, params| self.references(id, params, &token))?
                        .on::<HoverRequest, _>(|id, params| self.hover(id, params, &token))?
                        .on::<DocumentSymbolRequest, _>(|id, params| {
                            self.document_symbols(id, params, &token)
                        })?
                        .on::<WorkspaceSymbol, _>(|id, params| {
                            self.workspace_symbols(id, params, &token)
                        })?
                        .on::<Completion, _>(|id, params| {
                            #[cfg(feature = "completion")]
                            self.completion(id, params, &token)?;
                            Ok(())
                        })?
                        .on::<ResolveCompletionItem, _>(|id, params| {
                            #[cfg(feature = "completion")]
                            self.completion_resolve(id, params, &token)?;
                            Ok(())
                        })?
                        .on::<GotoDefinition, _>(|id, params| {
                            self.goto_definition(id, params, &token)
                        })?
                        .on::<PrepareRenameRequest, _>(|id, params| {
                            self.prepare_rename(id, params, &token)
                        })?
                        .on::<Rename, _>(|id, params| self.rename(id, params, &token))?
                        .on::<DocumentHighlightRequest, _>(|id, params| {
                            self.document_highlight(id, params, &token)
                        })?
                        .on::<Formatting, _>(|id, params| self.formatting(id, params, &token))?
                        .on::<BuildRequest, _>(|id, params| self.build(id, params, &token))?
                        .on::<ForwardSearchRequest, _>(|id, params| {
                            self.forward_search(id, params, &token)
                        })?
                        .on::<SemanticTokensRangeRequest, _>(|id, params| {
                            self.semantic_tokens_range(id, params, &token)
                        })?
                        .default()
                    {
                        self.connection.sender.send(response.into())?;
                    }
                }
                Message::Notification(notification) => {
                    NotificationDispatcher::new(notification)
                        .on::<Cancel, _>(|params| self.cancel(params))?
                        .on::<DidChangeConfiguration, _>(|params| {
                            self.did_change_configuration(params)
                        })?
                        .on::<DidOpenTextDocument, _>(|params| self.did_open(params))?
                        .on::<DidChangeTextDocument, _>(|params| self.did_change(params))?
                        .on::<DidSaveTextDocument, _>(|params| self.did_save(params))?
                        .on::<DidCloseTextDocument, _>(|params| self.did_close(params))?
                        .default();
                }
                Message::Response(response) => {
                    let mut req_queue = self.req_queue.lock().unwrap();
                    let data = req_queue.outgoing.complete(response.id);
                    let result = match response.result {
                        Some(result) => Ok(result),
                        None => Err(response
                            .error
                            .expect("response without result or error received")),
                    };
                    data.sender.send(result)?;
                }
            }
        }
        Ok(())
    }

    pub fn run(mut self) -> Result<()> {
        self.initialize()?;
        self.process_messages()?;
        drop(self.static_debouncer);
        drop(self.chktex_debouncer);
        self.pool.join();
        Ok(())
    }
}

fn create_static_debouncer(
    manager: Arc<Mutex<DiagnosticsManager>>,
    conn: &Connection,
    context: Arc<ServerContext>,
) -> DiagnosticsDebouncer {
    let sender = conn.sender.clone();
    DiagnosticsDebouncer::launch(context, move |workspace, document| {
        let mut manager = manager.lock().unwrap();
        manager.update_static(workspace.as_ref(), Arc::clone(&document.uri));
        if let Err(why) = publish_diagnostics(&sender, workspace.as_ref(), &manager) {
            warn!("Failed to publish diagnostics: {}", why);
        }
    })
}

fn create_chktex_debouncer(
    manager: Arc<Mutex<DiagnosticsManager>>,
    conn: &Connection,
    context: Arc<ServerContext>,
) -> DiagnosticsDebouncer {
    let sender = conn.sender.clone();
    DiagnosticsDebouncer::launch(Arc::clone(&context), move |workspace, document| {
        let options = { context.options.read().unwrap().clone() };
        let mut manager = manager.lock().unwrap();
        manager.update_chktex(workspace.as_ref(), Arc::clone(&document.uri), &options);
        if let Err(why) = publish_diagnostics(&sender, workspace.as_ref(), &manager) {
            warn!("Failed to publish diagnostics: {}", why);
        }
    })
}

fn publish_diagnostics(
    sender: &Sender<lsp_server::Message>,
    workspace: &dyn Workspace,
    diag_manager: &DiagnosticsManager,
) -> Result<()> {
    for document in workspace.documents() {
        let diagnostics = diag_manager.publish(Arc::clone(&document.uri));
        send_notification::<PublishDiagnostics>(
            sender,
            PublishDiagnosticsParams {
                uri: document.uri.as_ref().clone().into(),
                version: None,
                diagnostics,
            },
        )?;
    }
    Ok(())
}

fn cancel_response(id: RequestId) -> lsp_server::Response {
    lsp_server::Response::new_err(
        id,
        ErrorCode::RequestCanceled as i32,
        "canceled by client".to_string(),
    )
}

struct BuildRequest;

impl lsp_types::request::Request for BuildRequest {
    type Params = BuildParams;

    type Result = BuildResult;

    const METHOD: &'static str = "textDocument/build";
}

struct ForwardSearchRequest;

impl lsp_types::request::Request for ForwardSearchRequest {
    type Params = TextDocumentPositionParams;

    type Result = ForwardSearchResult;

    const METHOD: &'static str = "textDocument/forwardSearch";
}

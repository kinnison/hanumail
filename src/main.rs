use std::collections::HashMap;

use tokio::sync::Mutex;
use tower_lsp::{
    Client, LanguageServer, LspService, Server,
    jsonrpc::Result,
    lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, DocumentRangeFormattingParams, InitializeParams,
        InitializeResult, InitializedParams, MessageType, OneOf, ServerCapabilities, ServerInfo,
        TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
    },
};

#[derive(Debug)]
struct Backend {
    client: Client,
    documents: Mutex<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "Hanumail".into(),
                ..Default::default()
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let mut docs = self.documents.lock().await;
        docs.insert(
            params.text_document.uri.clone(),
            params.text_document.text.clone(),
        );
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut docs = self.documents.lock().await;
        *docs.entry(params.text_document.uri.clone()).or_default() =
            params.content_changes[0].text.clone();
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut docs = self.documents.lock().await;
        docs.remove(&params.text_document.uri);
    }

    async fn formatting(&self, _params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        Ok(None)
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let mut docs = self.documents.lock().await;
        let range = params.range;
        // eprintln!("{range:?}");
        let Some(doc) = docs.get_mut(&params.text_document.uri) else {
            return Ok(None);
        };

        let mut content = doc
            .lines()
            .skip(range.start.line as usize)
            .take((range.end.line - range.start.line + 1) as usize)
            .collect::<Vec<_>>();

        if content.is_empty() {
            return Ok(None);
        }

        // eprintln!("Before: {content:?}");
        content[0] = &content[0][range.start.character as usize..];
        let last = content.len() - 1;
        content[last] = &content[last][..range.end.character as usize];
        // eprintln!("after: {content:?}");

        let leading_blanks = content.iter().take_while(|s| s.is_empty()).count();
        let trailing_blanks = content.iter().rev().take_while(|s| s.is_empty()).count();
        // eprintln!("{leading_blanks} {trailing_blanks}");

        let new_text = reformat(&content);
        let mut ret = String::new();
        (0..leading_blanks).for_each(|_| ret.push('\n'));
        ret.push_str(new_text.trim());
        (0..trailing_blanks).for_each(|_| ret.push('\n'));
        Ok(Some(vec![TextEdit {
            range,
            new_text: ret,
        }]))
    }
}

// Reformat the input string, being intelligent about quoting
// so lines which start with `> *` get grouped.  Essentially
// we track a "level" for each line and then flow groups
// at the same level, and replace things to look neat
fn reformat(input: &[&str]) -> String {
    let input = input
        .into_iter()
        .map(|l| {
            let level = l
                .chars()
                .take_while(|&c| c == ' ' || c == '>')
                .filter(|&c| c == '>')
                .count();
            let rest = l
                .chars()
                .skip_while(|&c| c == ' ' || c == '>')
                .collect::<String>();
            (level, rest)
        })
        .collect::<Vec<_>>();
    // eprintln!("{input:?}");
    let mut ret = String::new();

    let mut acc = String::new();
    let mut curlevel = None;
    for (level, part) in input {
        // eprintln!("RET: {ret:?}");
        // eprintln!("HAVE: {acc:?} {curlevel:?}");
        // eprintln!("GOT: {level} {part}");
        if curlevel.is_none() {
            curlevel = Some(level);
            acc = part;
            continue;
        }
        if curlevel == Some(level) {
            acc.push(' ');
            acc.push_str(&part);
            continue;
        }
        do_wrap(&mut ret, &acc, curlevel);
        if curlevel == Some(0) || level == 0 {
            ret.push('\n');
        }
        curlevel = Some(level);
        acc = part.to_string();
    }

    // eprintln!("{acc:?} {curlevel:?}");
    if !acc.is_empty() {
        do_wrap(&mut ret, &acc, curlevel)
    }

    ret
}

fn do_wrap(ret: &mut String, acc: &String, curlevel: Option<usize>) {
    fn push_level(acc: &mut String, level: usize) {
        for _ in 0..level {
            acc.push('>');
            acc.push(' ');
        }
    }
    fn space(level: usize) -> usize {
        78 - (level * 2)
    }
    let mut lineacc = String::new();
    // eprintln!("Wrapping {acc:?} at level {curlevel:?}");
    for word in acc
        .trim()
        .split_ascii_whitespace()
        .skip_while(|s| s.is_empty())
    {
        lineacc = lineacc.trim().into();
        if lineacc.len() + word.len() + 1 > space(curlevel.unwrap()) {
            push_level(ret, curlevel.unwrap());
            ret.push_str(&lineacc);
            ret.push('\n');
            lineacc = word.into();
        } else {
            lineacc.push(' ');
            lineacc.push_str(word);
        }
    }
    if !lineacc.is_empty() {
        lineacc = lineacc.trim().into();
        push_level(ret, curlevel.unwrap());
        ret.push_str(&lineacc);
        ret.push('\n');
    }
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
        }
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend::new(client));

    Server::new(stdin, stdout, socket).serve(service).await;
}

use std::collections::HashMap;

use tokio::sync::Mutex;
use tower_lsp::{
    jsonrpc::Result,
    lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, DocumentRangeFormattingParams, InitializeParams,
        InitializeResult, InitializedParams, MessageType, OneOf, Position, Range,
        ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
        Url,
    },
    Client, LanguageServer, LspService, Server,
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

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let mut docs = self.documents.lock().await;
        let doc = docs.entry(params.text_document.uri.clone()).or_default();

        let whole_doc = Range::new(
            Position {
                line: 0,
                character: 0,
            },
            Position {
                line: doc.lines().count() as u32,
                character: 0,
            },
        );

        let new_doc = reformat_entire_doc(doc);

        Ok(Some(vec![TextEdit {
            range: whole_doc,
            new_text: new_doc,
        }]))
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
        .iter()
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
        let part = part.trim();
        if part.is_empty() {
            // Something already present, wrap that into the output
            if let Some(curlevel) = curlevel {
                do_wrap(&mut ret, &acc, curlevel);
            }
            curlevel = None;
            // Blank line
            do_wrap(&mut ret, "", level);
            continue;
        }
        match curlevel {
            None => {
                curlevel = Some(level);
                acc = part.to_string();
            }
            Some(cl) if cl == level => {
                acc.push(' ');
                acc.push_str(part);
            }
            Some(ol) => {
                do_wrap(&mut ret, &acc, ol);
                curlevel = Some(level);
                acc = part.to_string();
            }
        }
    }

    if let Some(level) = curlevel {
        do_wrap(&mut ret, &acc, level);
    }

    ret
}

fn do_wrap(ret: &mut String, acc: &str, curlevel: usize) {
    let level_str = "> ".repeat(curlevel);
    const LINE_LENGTH: usize = 78;

    let mut lineacc = level_str.clone();
    for word in acc.trim().split_ascii_whitespace() {
        if lineacc.len() + word.len() > LINE_LENGTH && lineacc.len() > level_str.len() {
            ret.push_str(lineacc.trim());
            ret.push('\n');
            lineacc = level_str.clone();
        }
        lineacc.push_str(word);
        lineacc.push(' ');
    }

    ret.push_str(lineacc.trim());
    ret.push('\n');
}

fn reformat_entire_doc(body_s: &str) -> String {
    enum ParseState {
        Header,
        Body,
        Signature,
    }
    use ParseState::*;
    let mut header = vec![];
    let mut body = vec![];
    let mut sig = vec![];

    let mut state = Header;
    for line in body_s.lines() {
        match state {
            Header => {
                header.push(line);
                if line.is_empty() {
                    state = Body;
                }
            }
            Body => {
                if line == "--" || line == "-- " {
                    state = Signature;
                    sig.push(line);
                } else {
                    body.push(line);
                }
            }
            Signature => {
                sig.push(line);
            }
        }
    }
    let new_body = reformat(&body);
    let header = header.join("\n");
    let sig = sig.join("\n");
    format!("{header}\n{new_body}{sig}\n")
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

    let (service, socket) = LspService::new(Backend::new);

    Server::new(stdin, stdout, socket).serve(service).await;
}

mod runtime;
use runtime::RuntimeOwner;
use spur_embedding::{
    read_frame, write_frame, ErrorCode, ProtocolLimits, Reply, ReplyEnvelope, Request,
    RequestEnvelope, Session, PROTOCOL_VERSION,
};
use std::{
    error::Error,
    io::{self, Read, Write},
    path::PathBuf,
};

fn run() -> Result<(), Box<dyn Error>> {
    let values: Vec<usize> = std::env::args()
        .skip(1)
        .map(|x| x.parse())
        .collect::<Result<_, _>>()?;
    let [frame, texts, text_bytes, total_bytes, dimensions, identity_bytes, pending] =
        values.as_slice()
    else {
        return Err("expected seven explicit protocol limits".into());
    };
    let limits = ProtocolLimits {
        max_frame_bytes: u32::try_from(*frame)?,
        max_texts: *texts,
        max_text_bytes: *text_bytes,
        max_total_text_bytes: *total_bytes,
        max_dimensions: *dimensions,
        max_identity_bytes: *identity_bytes,
        max_pending: *pending,
    };
    limits.validate()?;
    let cache = std::env::var_os("SPUR_ORT_CACHE")
        .map(PathBuf::from)
        .ok_or("SPUR_ORT_CACHE is required")?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let hello: RequestEnvelope = read_frame(&mut input, &limits)?;
    let Request::Hello { identity } = &hello.request else {
        return Err("expected Hello".into());
    };
    let mut session = Session::new(hello.generation, identity.clone(), limits.clone())?;
    session.validate_request(&hello)?;
    let mut owner = match RuntimeOwner::start(&cache) {
        Ok(owner) => owner,
        Err(error) => {
            let response = ReplyEnvelope {
                version: PROTOCOL_VERSION,
                generation: hello.generation,
                request_id: hello.request_id,
                reply: Reply::Error {
                    code: ErrorCode::Unavailable,
                    message: "native initialization failed"
                        .chars()
                        .take(limits.max_identity_bytes)
                        .collect(),
                },
            };
            session.complete_reply(&hello, &response)?;
            write_frame(&mut output, &response, &limits)?;
            output.flush()?;
            return Err(error.into());
        }
    };
    let result = serve(&limits, &mut owner, &mut input, &mut output, session, hello);
    let close = owner.close();
    close?;
    result
}
fn serve(
    limits: &ProtocolLimits,
    owner: &mut RuntimeOwner,
    input: &mut impl Read,
    output: &mut impl Write,
    mut session: Session,
    hello: RequestEnvelope,
) -> Result<(), Box<dyn Error>> {
    let mut first = Some(hello);
    loop {
        let request = if let Some(hello) = first.take() {
            hello
        } else {
            let request = read_frame(&mut *input, limits)?;
            session.validate_request(&request)?;
            request
        };
        let reply = match &request.request {
            Request::Hello { identity } => Reply::Ready {
                identity: identity.clone(),
            },
            Request::Embed { .. } => Reply::Error {
                code: ErrorCode::Unavailable,
                message: "product model loading is unavailable"
                    .chars()
                    .take(limits.max_identity_bytes)
                    .collect(),
            },
            Request::Shutdown => {
                owner.close()?;
                Reply::Closed
            }
        };
        let shutdown = matches!(reply, Reply::Closed);
        let response = ReplyEnvelope {
            version: PROTOCOL_VERSION,
            generation: request.generation,
            request_id: request.request_id,
            reply,
        };
        session.complete_reply(&request, &response)?;
        write_frame(&mut *output, &response, limits)?;
        output.flush()?;
        if shutdown {
            return Ok(());
        }
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    run()
}

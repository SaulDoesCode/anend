//! The web build's backend: the hub's app API over a WebSocket at `/app/ws`
//! (docs/app-api.md). Calls send `{id, op, args}`; answers come back as
//! `{id, ok, data | error}`, matched by id. The socket opens on the first call and again
//! after it drops; calls made meanwhile wait in a queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures_channel::oneshot;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use super::{Backend, Fut};

type Answer = oneshot::Sender<Result<Value, String>>;

#[derive(Default)]
struct State {
    ws: Option<WebSocket>,
    open: bool,
    queue: Vec<String>,
    pending: HashMap<u64, Answer>,
    next: u64,
}

#[derive(Clone)]
pub struct Hub {
    url: String,
    st: Rc<RefCell<State>>,
}

impl Hub {
    /// the hub that served this page
    pub fn here() -> Hub {
        let loc = web_sys::window().map(|w| w.location());
        let (proto, host) = loc
            .map(|l| (l.protocol().unwrap_or_default(), l.host().unwrap_or_default()))
            .unwrap_or_default();
        let ws = if proto == "https:" { "wss" } else { "ws" };
        Hub {
            url: format!("{ws}://{host}/app/ws"),
            st: Default::default(),
        }
    }

    fn connect(&self) -> Result<(), String> {
        if self.st.borrow().ws.is_some() {
            return Ok(());
        }
        let ws = WebSocket::new(&self.url).map_err(|e| format!("couldn't reach the hub: {e:?}"))?;
        let st = self.st.clone();
        let onopen = Closure::<dyn FnMut()>::new(move || {
            let mut s = st.borrow_mut();
            s.open = true;
            let queued: Vec<String> = s.queue.drain(..).collect();
            if let Some(ws) = &s.ws {
                for m in queued {
                    let _ = ws.send_with_str(&m);
                }
            }
        });
        let st = self.st.clone();
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(text) = e.data().as_string() else { return };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                return;
            };
            let id = v["id"].as_u64().unwrap_or(0);
            if let Some(tx) = st.borrow_mut().pending.remove(&id) {
                let r = if v["ok"].as_bool() == Some(true) {
                    Ok(v.get("data").cloned().unwrap_or(Value::Null))
                } else {
                    Err(v["error"].as_str().unwrap_or("the hub said no").to_string())
                };
                let _ = tx.send(r);
            }
        });
        let st = self.st.clone();
        let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |_e: CloseEvent| {
            let mut s = st.borrow_mut();
            s.ws = None;
            s.open = false;
            for (_, tx) in s.pending.drain() {
                let _ = tx.send(Err("the hub dropped the connection; try again".into()));
            }
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        // they live as long as the socket
        onopen.forget();
        onmessage.forget();
        onclose.forget();
        self.st.borrow_mut().ws = Some(ws);
        Ok(())
    }
}

impl Backend for Hub {
    fn call(&self, op: &str, args: Value) -> Fut<Value> {
        let (tx, rx) = oneshot::channel();
        let sent = self.connect().map(|_| {
            let mut s = self.st.borrow_mut();
            s.next += 1;
            let id = s.next;
            s.pending.insert(id, tx);
            let msg = json!({ "id": id, "op": op, "args": args }).to_string();
            match (&s.ws, s.open) {
                (Some(ws), true) => {
                    let _ = ws.send_with_str(&msg);
                }
                _ => s.queue.push(msg),
            }
        });
        Box::pin(async move {
            sent?;
            rx.await.map_err(|_| "the hub didn't answer".to_string())?
        })
    }

    fn describe(&self) -> String {
        format!("the hub at {}", self.url)
    }
}

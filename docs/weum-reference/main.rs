// #![allow(dead_code)]
#![feature(portable_simd)]
#![feature(string_into_chars)]

use anyhow::anyhow;// use moka::future::Cache;
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, Debouncer, NoCache, new_debouncer};
use rand::{Rng, RngExt, distr::Alphanumeric};
use redb::{Database, MultimapTableDefinition, MultimapValue, ReadableDatabase, ReadableMultimapTable, ReadableTable, ReadableTableMetadata, TableDefinition};
use rust_decimal::prelude::*;
use salvo::{
    conn::rustls::{Keycert, RustlsConfig},
    fs::NamedFile,
    http::cookie::Cookie,
    prelude::*,
    serve_static::StaticDir, //websocket::{Message, WebSocketUpgrade},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap, env, fs::{self}, io::Read, path::{Path}, 
    process::{Command}, simd::{cmp::SimdPartialEq, prelude::*},
    sync::{LazyLock, OnceLock, atomic::AtomicU8, mpsc}, time::{Duration, SystemTime, UNIX_EPOCH}
};
use tokio::sync::broadcast;
use sthash::*;
use rayon::prelude::*;
use simsimd::SpatialSimilarity;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use fastembed::{Qwen3VLEmbedding};
use candle_core::{DType, Device};

fn qwen3_embedder() -> anyhow::Result<Qwen3VLEmbedding> {
    Ok(Qwen3VLEmbedding::from_hf(
        "Qwen/Qwen3-VL-Embedding-2B",
        &if let Ok(d) = Device::cuda_if_available(0) {
            println!("took the cuda option for embedding acceleration");
            d 
        } else { Device::Cpu },
        DType::F32,
        2048,
    )?)
}

static EMBEDDER: LazyLock<Qwen3VLEmbedding> = LazyLock::new(|| qwen3_embedder().expect("embbeder couldn't get started"));

const EMBEDDINGS: TableDefinition<&[u8], Vec<f32>> = TableDefinition::new("embeddings");

fn disambiguate_embedding_kind(query: &str) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(if query.ends_with(".png") || query.ends_with(".jpg") {
        EMBEDDER.embed_images(&[query])?
    } else {
        EMBEDDER.embed_texts(&[query])?
    }.pop())
}

fn search_embeddings(query: &str, sensitivity: f64, limit: usize, mut also_saturate: bool) -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let rx = MEMORY.begin_read()?;
    let embeddings = rx.open_table(EMBEDDINGS)?;
    let qb = query.as_bytes();
    let mut already_saturated = false;
    let bedding: Option<Vec<f32>> = if let Ok((fnd, _)) = memory_search(qb, false) { // this is better than having a sha256 conflict lookup, reusing search for that is like, a dual use
        if let Some((k, _)) = fnd.iter().find(|t| qb.eq(t.1)) { // fnd.par_iter().find_any(|t| qb.eq(t.1)) {
            let b = embeddings.get(k.as_slice())?.map(|ag| ag.value());
            if b.is_none() {
                disambiguate_embedding_kind(query)?
            } else {
                already_saturated = true;
                b
            }
        } else {
            disambiguate_embedding_kind(query)? 
        }
    } else {
        disambiguate_embedding_kind(query)?
    };
    if let Some(query_embedding) = bedding {
        let mut found = vec![]; /*let mut found: Vec<Vec<u8>> = embeddings.iter()?.par_bridge().take_any(limit).filter_map(|r | { match r { Ok((k_ag, v_ag)) => { if let Some(distance) = f32::cosine(&query_embedding, &v_ag.value()) { if (1.0 - distance) >= sensitivity { return Some(k_ag.value().to_vec()); } } }, Err(e) => { println!("storage err, oof: {}", e); } } None }).collect();*/ 
        let mut iter = embeddings.iter()?;
        while let Some(r) = iter.next() { 
            match r { 
                Ok((k_ag, v_ag)) => { 
                    if let Some(distance) = f32::cosine(&query_embedding, &v_ag.value()) { 
                        if (1.0 - distance) >= sensitivity { found.push(k_ag.value().to_vec());
                        if found.len() >= limit { break; } } } 
                    },
                    Err(e) => { println!("storage err, oof: {}", e); }
                } 
        }
        if !already_saturated && also_saturate { // huam
            let r = saturate(qb, None);
            if let Err(e) = r {
                println!("unsaturative esearch {}", e);
                also_saturate = false;
            } else {
                println!("saturative search is done for {:?}", r.unwrap());
            }
        }
        if found.len() == 0 {
            return Err(if also_saturate && !already_saturated { // nam
                anyhow!("embedding search was unsuccessful, but saturated") 
            } else {
                anyhow!("embedding search was unsuccessful")
            });
        }
        let memories = rx.open_table(MEMORIES)?;
        let mut hm = HashMap::new();
        for f in found.drain(..) {
            match memories.get(f.as_slice()).map(|o| o.map(|ag| ag.value().to_vec())) {
                Ok(Some(v)) => { hm.insert(f, v); }
                Ok(None) => { continue; }
                Err(e) => {
                    println!("err in memory read, {}", e);
                    continue;
                },
            }
        }
        Ok(hm)
    } else { Err(anyhow!("failed to generate an embedding for the query")) }
}

static DB: LazyLock<Database> = LazyLock::new(|| Database::create("./re.db").expect("opening/creating db had issues"));

fn unix_now(dur: u64) -> anyhow::Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + dur)
}

const ACCOUNTS: TableDefinition<
    u64, // id
    (
        String,      // moniker
        Vec<u8>,     // pwd hash
        String,      // email
        String,      // description
        Vec<String>, // tags
        String,      // balance &str to Decimal needed
    ),
> = TableDefinition::new("accounts");

const PWD_HASH_LOOKUP: TableDefinition<(&str, &[u8]), u64> = TableDefinition::new("pwd_lookup");
const SESSIONS_LOOKUP: MultimapTableDefinition<
    u64,   // acc id
    &[u8], // session id hashes
> = MultimapTableDefinition::new("sessions lookup"); // sessions (for revokability, backward lookup)

fn make_account(
    moniker: &str,
    email: &str,
    pwd: &str,
    desc: &str,
    tags: Vec<String>,
) -> anyhow::Result<u64> {
    let id: u64 = unix_now(0)?;
    let pwd_hash = acc_hashr()?.hash(pwd.as_bytes());
    let wtx = DB.begin_write()?;
    {
        let mut accs = wtx.open_table(ACCOUNTS)?;
        if accs.get(id)?.is_some() {
            return Err(anyhow!(
                "some how someone else made an account the same second, and that messed things up, kindly try again"
            ));
        }
        let mut pl = wtx.open_table(PWD_HASH_LOOKUP)?;
        pl.insert((email, pwd_hash.as_slice()), id)?;
        accs.insert(
            id,
            &(
                moniker.to_string(),
                pwd_hash,
                email.to_string(),
                desc.to_string(),
                tags,
                "0".to_string(),
            ),
        )?;
    }
    wtx.commit()?;
    Ok(id)
}

fn expiry_check() -> anyhow::Result<()> {
    let n = unix_now(0)?;
    let wx = DB.begin_write()?;
    let mut to_remove = vec![];
    {
        wx.open_table(EXPIRIES_SESSIONS)?.retain_in(n.., |_, s| {
            to_remove.push(s.to_vec());
            false
        })?;
        let mut sess = wx.open_table(SESSIONS)?;
        let mut sl = wx.open_multimap_table(SESSIONS_LOOKUP)?;
        for s in to_remove {
            let sas = s.as_slice();
            if let Some(ag) = sess.remove(sas)? {
                sl.remove(ag.value().0, sas)?;
            }
        }
    }
    wx.commit()?;
    Ok(())
}

static EXPIRE_EXPIRY: AtomicU8 = AtomicU8::new(0);

fn expiry_thread() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {
        while EXPIRE_EXPIRY.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            if let Err(e) = expiry_check() {
                println!("expiry check had issues: {}", e);
            }
            std::thread::sleep(Duration::from_secs(30));
        }
        return ();
    }) // todo lazy/once lock moka cache
}

fn acc_id_by_session_hash(sid_hashed: Vec<u8>) -> anyhow::Result<u64> {
    let n = unix_now(0)?;
    if let Some(ag) = DB
        .begin_read()?
        .open_table(SESSIONS)?
        .get(sid_hashed.as_slice())?
    {
        let (id, exp) = ag.value();
        return if n < exp {
            Ok(id)
        } else {
            Err(anyhow!("session expired"))
        };
    }
    Err(anyhow!("didn't find that account, no session matched"))
}

#[handler]
async fn register(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let details: (&str, &str, &str) = req.parse_body().await?;
    let id = make_account(details.0, details.1, details.2, "", vec![])?;
    let sid = make_session_for(id, unix_now(60 * 60 * 24 * 31)?)?;
    res.add_cookie(Cookie::new("auth", sid));
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut()
        .insert("Content-Type", "text/plain".parse().unwrap());
    res.body("registered");
    Ok(())
}

fn acc_id_from_email_and_pwd(email: &str, pwd: &str) -> anyhow::Result<u64> {
    match DB
        .begin_read()?
        .open_table(PWD_HASH_LOOKUP)?
        .get((email, acc_hashr()?.hash(pwd.as_bytes()).as_slice()))?
    {
        Some(ag) => Ok(ag.value()),
        None => Err(anyhow!("unable to authenticate details")),
    }
}

#[handler]
async fn signin(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let (email, pwd) = req.parse_body_with_max_size(512).await?;
    res.add_cookie(Cookie::new(
        "auth",
        make_session_for(
            acc_id_from_email_and_pwd(email, pwd)?,
            unix_now(60 * 60 * 24 * 31)?, // ts qua id implies: ratelimit-already | moo
        )?,
    ));
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut()
        .insert("Content-Type", "text/plain".parse().unwrap());
    res.body("signed in");
    Ok(())
}

#[handler]
async fn signup(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let details: (&str, &str, &str) = req.parse_body_with_max_size(1024 - 256).await?;
    let id = make_account(details.0, details.1, details.2, "", vec![])?;
    res.add_cookie(Cookie::new(
        "auth",
        make_session_for(id, unix_now(60 * 60 * 24 * 31)?)?,
    ));
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut()
        .insert("Content-Type", "text/plain".parse().unwrap());
    res.body("signed up");
    Ok(())
}

/*
#[handler]
async fn signout(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let (id, sid_hashed) = auth_check_with_sid_hashed(req, res)?;
    res.remove_cookie("auth");
    let wtx = DB.begin_write()?;
    {
        let mut sess = wtx.open_table(SESSIONS)?;
        let mut exp_sess = wtx.open_table(EXPIRIES_SESSIONS)?;
        let mut sl = wtx.open_multimap_table(SESSIONS_LOOKUP)?;
        sl.remove(id, sid_hashed.as_slice())?;
        if let Some(ag) = sess.remove(sid_hashed.as_slice())? {
            exp_sess.remove(ag.value().1)?;
        }
    }
    wtx.commit()?;
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut()
        .insert("Content-Type", "text/plain".parse().unwrap());
    res.body("signed out");
    Ok(())
}*/

fn auth_check(req: &mut Request, res: &mut Response) -> anyhow::Result<u64> {
    if let Some(c) = req.cookie("auth") {
        return Ok(acc_id_by_session_hash(
            acc_hashr()?.hash(c.value().as_bytes()),
        )?);
    }
    res.status_code(StatusCode::UNAUTHORIZED);
    Err(anyhow!("didn't pass the auth check"))
}

fn auth_check_admin(req: &mut Request, res: &mut Response) -> anyhow::Result<u64> {
    if let Some(c) = req.cookie("auth") {
        let id = acc_id_by_session_hash(acc_hashr()?.hash(c.value().as_bytes()))?;
        if let Some(ag) = DB.begin_read()?.open_table(ACCOUNTS)?.get(id)? {
            if ag.value().4.contains(&"admin".to_string()) {
                return Ok(id);
            }
        }
    }
    res.status_code(StatusCode::UNAUTHORIZED);
    Err(anyhow!("didn't pass the admin check"))
}

fn auth_check_and_is_admin(req: &mut Request, res: &mut Response) -> anyhow::Result<(u64, bool)> {
    if let Some(c) = req.cookie("auth") {
        let id = acc_id_by_session_hash(acc_hashr()?.hash(c.value().as_bytes()))?;
        if let Some(ag) = DB.begin_read()?.open_table(ACCOUNTS)?.get(id)? {
            if ag.value().4.contains(&"admin".to_string()) {
                return Ok((id, true));
            }
            return Ok((id, false));
        }
    }
    res.status_code(StatusCode::UNAUTHORIZED);
    Err(anyhow!("didn't pass the admin check"))
}

fn auth_check_with_sid_hashed(
    req: &mut Request,
    res: &mut Response,
) -> anyhow::Result<(u64, Vec<u8>)> {
    if let Some(c) = req.cookie("auth") {
        let sid_hashed = acc_hashr()?.hash(c.value().as_bytes());
        return Ok((acc_id_by_session_hash(sid_hashed.clone())?, sid_hashed));
    }
    res.status_code(StatusCode::UNAUTHORIZED);
    Err(anyhow!("didn't pass the auth check"))
}

#[handler]
async fn make_admin(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let pwd: String = req.parse_body_with_max_size(256).await?;
    let id = auth_check(req, res)?;
    if pwd == tokio::fs::read_to_string("./certs/admin_pwd").await? {
        let wx = DB.begin_write()?;
        {
            let mut accs = wx.open_table(ACCOUNTS)?;
            let mut oacc = None;
            if let Some(ag) = accs.get(id)? {
                let mut acc = ag.value();
                acc.4.push("admin".to_string());
                oacc = Some(acc);
            }
            if let Some(acc) = oacc {
                accs.insert(id, acc)?;
            } else {
                res.status_code(StatusCode::UNAUTHORIZED);
                res.render(Text::Plain("not adminized"));
                return Err(anyhow!("not adminizable like this"));
            }
        }
        wx.commit()?;
        res.render(Text::Plain("adminized"));
    }
    Ok(())
}

#[handler]
async fn unregister(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    rm_account(auth_check(req, res)?)?;
    res.status_code(StatusCode::ACCEPTED);
    res.body("ciao");
    Ok(())
}

/*fn rm_account(id: u64) -> anyhow::Result<()> {
    let wtx = DB.begin_write()?;
    {
        let mut accs = wtx.open_table(ACCOUNTS)?;
        if let Some(_ag) = accs.remove(id)? {
            let mut sess = wtx.open_table(SESSIONS)?;
            let mut exp_sess = wtx.open_table(EXPIRIES_SESSIONS)?;
            let mut sl = wtx.open_multimap_table(SESSIONS_LOOKUP)?;
            let mut iter = sl.get(id)?;
            while let Some(Ok(nx)) = iter.next() {
                if let Some(ag) = sess.remove(nx.value())? {
                    exp_sess.remove(ag.value().1)?;
                }
            }
            drop(iter);
            sl.remove_all(id)?;
        }
    }
    wtx.commit()?;
    Ok(())
}*/

const SESSIONS: TableDefinition<
    &[u8],      // session hash
    (u64, u64), // acc id, expiry timepstamp
> = TableDefinition::new("sessions"); /* timestamp, id to be expired ( entry exists implies not yet expired, background culling process prevents having to check) */

const EXPIRIES_SESSIONS: TableDefinition<
    u64,   // expiry
    &[u8], // session id hash
> = TableDefinition::new("expiries_sessions");

fn acc_hashr() -> anyhow::Result<sthash::Hasher> {
    establish_hashr(Some("acc"), Some("shop"))
}

fn make_session_for(id: u64, exp: u64) -> anyhow::Result<String> {
    let hashr = acc_hashr()?;
    let wtx = DB.begin_write()?;
    let sid = randstr(24);
    let mut abort = false;
    {
        let mut sess = wtx.open_table(SESSIONS)?;
        if let Some(ag) = sess.insert(hashr.hash(sid.as_bytes()).as_slice(), &(id, exp))? {
            let v = ag.value();
            abort = id != v.0;
        }
    }
    if abort {
        wtx.abort()?;
        return Err(anyhow!("had to abort due to a rare collision, try again"));
    }
    wtx.commit()?;
    Ok(sid)
}

const ORDERS: TableDefinition<
    &str, // reference/order_id
    (
        u64,                // account id
        Vec<(String, u32)>, // products as id and quantity stock ordered
        String,             // shipping address
        String,             // total
        u8, // status, 0 = AwaitingPayment, 1 = Paid, 2 = Dispatched, 3 = Cancelled, 4 = Refunded, 5 Processing
        Vec<u64>, // timestamps of modification, first one when order is made
        Vec<String>, // tags
    ),
> = TableDefinition::new("orders");

const ACC_ORDER_LOOKUP: MultimapTableDefinition<u64, &str> =
    MultimapTableDefinition::new("acc order lookup");

fn make_order(
    product_list: Vec<(String, u32)>,
    acc_id: u64,
    shipping_address: String,
    tags: Vec<String>,
) -> anyhow::Result<(
    Decimal,
    String,
    Vec<(u32, u8, String, String, Vec<String>, Vec<String>)>,
)> {
    let (total, products) = retrieve_products(&product_list)?;
    let order_id = randstr(12);
    let status = 0u8;
    let n = unix_now(0)?;
    let wx = DB.begin_write()?;
    {
        let mut orders = wx.open_table(ORDERS)?;
        wx.open_multimap_table(ACC_ORDER_LOOKUP)?
            .insert(acc_id, order_id.as_str())?;
        orders.insert(
            order_id.as_str(),
            &(
                acc_id,
                product_list,
                shipping_address,
                total.to_string(),
                status,
                vec![n],
                tags,
            ),
        )?;
    }
    wx.commit()?;
    Ok((total, order_id, products))
}

#[handler]
async fn place_order(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let id = auth_check(req, res)?;
    let (product_list, shipping_address) = req.parse_body_with_max_size(16596).await?;
    let o = make_order(product_list, id, shipping_address, vec![])?;
    res.headers_mut()
        .insert("Content-Type", "application/json".parse().unwrap());
    // TODO send invoice and have non manual bank transfer based payment
    res.body(serde_json::to_string(&o)?);
    Ok(())
}

#[handler]
async fn tag_order(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let (order_id, tags): (String, Vec<String>) = req.parse_body_with_max_size(16596).await?;
    let wx = DB.begin_write()?;
    let mut order = None;
    {
        let mut orders = wx.open_table(ORDERS)?;
        if let Some(ag) = orders.get(order_id.as_str())? {
            order = Some(ag.value());
        }
        if let Some(o) = &mut order {
            o.6.extend_from_slice(&tags);
            orders.insert(order_id.as_str(), o)?;
        }
    }
    wx.commit()?;
    if let Some(o) = order {
        res.headers_mut()
            .insert("Content-Type", "application/json".parse().unwrap());
        res.body(serde_json::to_string(&o)?);
        return Ok(());
    }
    res.status_code(StatusCode::FORBIDDEN);
    Err(anyhow!(
        "tag setting on order didn't work, could be that one doesn't exist"
    ))
}

/*
fn remove_order(
    id: &str,
) -> anyhow::Result<(
    u64,                // account id
    Vec<(String, u32)>, // products as id and quantity stock ordered
    String,             // shipping address
    String,             // total
    u8, // status, 0 = AwaitingPayment, 1 = Paid, 2 = Dispatched, 3 = Cancelled, 4 = Refunded, 5 = Processing
    Vec<u64>, // timestamps of modification, first one when order is made
    Vec<String>, // tags
)> {
    let wx = DB.begin_write()?;
    let mut o_order = None;
    {
        let mut orders = wx.open_table(ORDERS)?;
        if let Some(ag) = orders.remove(id)? {
            let o = ag.value();
            wx.open_multimap_table(ACC_ORDER_LOOKUP)?.remove(o.0, id)?;
            o_order = Some(o);
        }
        if let Some(order) = &o_order {
            orders.insert(id, order)?;
        }
    }
    wx.commit()?;
    match o_order {
        Some(o) => Ok(o),
        None => Err(anyhow!(
            "unable to remove order, it did not exist in the db"
        )),
    }
}*/

#[handler]
async fn order_remove(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let id: String = req.parse_body_with_max_size(2048).await?;
    remove_order(&id)?;
    res.status_code(StatusCode::OK);
    Ok(())
}

#[handler]
async fn cancel_order(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let id = auth_check(req, res)?;
    let oid: String = req.parse_body_with_max_size(16596).await?;
    let wx = DB.begin_write()?;
    let mut o_order = None;
    let mut abort = false;
    {
        let mut orders = wx.open_table(ORDERS)?;
        if let Some(ag) = orders.get(oid.as_str())? {
            let o = ag.value();
            if o.0 != id {
                res.status_code(StatusCode::FORBIDDEN);
                res.body("that's not your order, or this is not your account.");
                abort = true;
            }
            o_order = Some(o);
        }
        if !abort {
            if let Some(order) = &mut o_order {
                let paid = order.4 != 1;
                let dispatched = order.4 != 3;
                if paid || dispatched {
                    order.4 = 3;
                    orders.insert(oid.as_str(), order)?;
                } else {
                    res.status_code(StatusCode::CONFLICT);
                    res.body(if paid {
                        "this order is paid for, next step is to contact us, it cannot be immediately cancelled as per store policy"
                    } else if dispatched {
                        "this order is dispatched already, if something is wrong with the product, contact us about a refund"
                    } else {
                        "something in the order process went terribly wrong"
                    });
                    abort = true;
                }
            }
        }
    }
    if abort {
        wx.abort()?;
        return Err(anyhow!("order cancellation was aborted, look into it"));
    } else {
        wx.commit()?;
        if let Some(o) = o_order {
            res.headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());
            res.status_code(StatusCode::OK);
            res.body(serde_json::to_string(&o)?);
        }
    }
    Ok(())
}

fn get_order(
    id: &str,
) -> anyhow::Result<(
    u64,
    Vec<(String, u32)>,
    String,
    String,
    u8,
    Vec<u64>,
    Vec<String>,
)> {
    if let Some(ag) = DB.begin_read()?.open_table(ORDERS)?.get(id)? {
        return Ok(ag.value());
    }
    Err(anyhow!("unable to find order"))
}

#[handler]
async fn view_order(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let acc_id = auth_check(req, res)?;
    let id = match req.param("id") {
        Some(id) => id,
        None => {
            res.status_code(StatusCode::BAD_REQUEST);
            return Err(anyhow!("product id param missing"));
        }
    };
    match get_order(id) {
        Ok(order) => {
            if order.0 != acc_id {
                res.status_code(StatusCode::FORBIDDEN);
                res.body("that's not your order, or this is not your account.");
                return Err(anyhow!(
                    "someone got the order number of an order that isn't theirs or they are on the wrong account"
                ));
            }
            res.headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());
            res.status_code(StatusCode::OK);
            res.body(serde_json::to_string(&order)?);
            Ok(())
        }
        Err(e) => {
            res.status_code(StatusCode::NOT_FOUND);
            println!("get_order err: {}", e);
            Err(anyhow!("didn't get order"))
        }
    }
}

fn get_orders(
    ids: Option<Vec<u64>>,
    limit: Option<usize>,
) -> anyhow::Result<
    Vec<(
        String,
        (
            u64,
            Vec<(String, u32)>,
            String,
            String,
            u8,
            Vec<u64>,
            Vec<String>,
        ),
    )>,
> {
    let mut orders = vec![];
    let rx = DB.begin_read()?;
    let otb = rx.open_table(ORDERS)?;
    let mut iter = otb.iter()?;
    while let Some(r) = iter.next() {
        match r {
            Ok((k_ag, v_ag)) => {
                let o = v_ag.value();
                if let Some(ids) = &ids {
                    if !ids.contains(&o.0) {
                        continue;
                    }
                }
                orders.push((k_ag.value().to_string(), o));
                if let Some(l) = &limit {
                    if l.ge(&orders.len()) {
                        break;
                    }
                }
            }
            Err(e) => println!(
                "issue fetching a particular order, cannot say which, see what is not there: {}",
                e
            ),
        }
    }
    Ok(orders)
}

#[handler]
async fn view_orders(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let (acc_id, is_admin) = auth_check_and_is_admin(req, res)?;
    match if is_admin {
        get_orders(
            if let Ok(these) = req.parse_body_with_max_size(8196).await {
                Some(these)
            } else {
                None
            },
            None,
        )
    } else {
        get_acc_orders(acc_id)
    } {
        Ok(orders) => {
            res.headers_mut()
                .insert("content-type", "application/json".parse().unwrap());
            res.status_code(StatusCode::OK);
            res.body(serde_json::to_string(&orders)?);
            Ok(())
        }
        Err(e) => {
            res.status_code(StatusCode::NOT_FOUND);
            println!("get_order err: {}", e);
            Err(anyhow!("didn't get order"))
        }
    }
}

fn get_acc_orders(
    acc_id: u64,
) -> anyhow::Result<
    Vec<(
        String,
        (
            u64,
            Vec<(String, u32)>,
            String,
            String,
            u8,
            Vec<u64>,
            Vec<String>,
        ),
    )>,
> {
    let rx = DB.begin_read()?;
    let aol = rx.open_multimap_table(ACC_ORDER_LOOKUP)?;
    let mut iter = aol.get(acc_id)?;
    let mut orders = vec![];
    let mut n = iter.next();
    loop {
        if let Some(r) = n {
            match r {
                Ok(ag) => orders.push(ag.value().to_string()),
                Err(e) => println!("acc order lookup error: {}", e),
            }
            n = iter.next();
        } else {
            break;
        }
    }
    let otb = rx.open_table(ORDERS)?;
    let mut res = vec![];
    for id in orders {
        if let Some(ag) = otb.get(id.as_str())? {
            res.push((id, ag.value()));
        }
    }
    Ok(res)
}

fn change_order_status(id: &str, status: u8) -> anyhow::Result<()> {
    let wx = DB.begin_write()?;
    {
        let mut orders = wx.open_table(ORDERS)?;
        let mut o_order = None;
        if let Some(ag) = orders.get(id)? {
            o_order = Some(ag.value());
        }
        if let Some(mut order) = o_order {
            order.4 = status;
            orders.insert(id, &order)?;
        }
    }
    Ok(())
}

#[handler]
async fn order_status_change(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let (id, status) = req.parse_body_with_max_size(2048).await?;
    change_order_status(id, status)?;
    res.status_code(StatusCode::OK);
    Ok(())
}

const PRODUCTS: TableDefinition<
    &str, // product name
    (u32, u8, String, String, Vec<String>, Vec<String>), /*
              stock,
              kind (0 = unique, 1 = normal...),
              description string,
              price as string unto decimal,
              image file paths for display,
              tags
          */
> = TableDefinition::new("products");

const PRICE_LOOKUP: MultimapTableDefinition<&str, &str> =
    MultimapTableDefinition::new("price lookup");

const TAG_LOOKUP: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("tag lookup");

fn insert_product(
    name: String,
    stock: u32,
    kind: u8, // 0 = normal, 1 = unique item, 2 = ?, 3 = ? ... 255 = no longer available
    description: String,
    price: Decimal,
    imgs: Vec<String>,
    tags: Vec<String>,
) -> anyhow::Result<()> {
    let wx = DB.begin_write()?;
    {
        let mut tl = wx.open_multimap_table(TAG_LOOKUP)?;
        for tag in &tags {
            tl.insert(tag.as_str(), name.as_str())?;
        }
        wx.open_table(PRODUCTS)?.insert(
            name.as_str(),
            &(stock, kind, description, price.to_string(), imgs, tags),
        )?;
        wx.open_multimap_table(PRICE_LOOKUP)?
            .insert(price.to_string().as_str(), name.as_str())?;
    }
    wx.commit()?;
    Ok(())
}

#[handler]
async fn product_insert(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let product: (String, u32, u8, String, Decimal, Vec<String>, Vec<String>) =
        req.parse_body_with_max_size(8192 * 2).await?;
    insert_product(
        product.0, product.1, product.2, product.3, product.4, product.5, product.6,
    )?;
    Ok(())
}

fn retrieve_products(
    items: &[(String, u32)],
) -> anyhow::Result<(
    Decimal,
    Vec<(u32, u8, String, String, Vec<String>, Vec<String>)>,
)> {
    let rx = DB.begin_read()?;
    let prs = rx.open_table(PRODUCTS)?;
    let mut res = vec![];
    let mut total = dec!(0);
    for (name, quantity) in items {
        if let Some(ag) = prs.get(name.as_str())? {
            let p = ag.value();
            if p.0 >= *quantity {
                total += Decimal::from_str_exact(&p.3)?;
                res.push(p);
            }
        }
    }
    Ok((total, res))
}

#[handler]
async fn get_product(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    if let Some(n) = req.param::<String>("name") {
        auth_check(req, res)?;
        if let Some(ag) = DB.begin_read()?.open_table(PRODUCTS)?.get(n.as_str())? {
            res.status_code(StatusCode::FOUND);
            res.body(serde_json::to_string(&ag.value())?);
            return Ok(());
        }
    }
    res.status_code(StatusCode::NOT_FOUND);
    Err(anyhow!("relevant product not found"))
}

fn remove_product(name: &str) -> anyhow::Result<()> {
    let wx = DB.begin_write()?;
    {
        if let Some(ag) = wx.open_table(PRODUCTS)?.remove(name)? {
            let p = ag.value();
            let mut tl = wx.open_multimap_table(TAG_LOOKUP)?;
            for tag in &p.5 { tl.remove(tag.as_str(), name)?; }
            wx.open_multimap_table(PRICE_LOOKUP)?
                .remove(p.3.as_str(), name)?;
        }
    }
    wx.commit()?;
    Ok(())
}

#[handler]
async fn product_remove(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    remove_product(req.parse_body_with_max_size(1024).await?)?;
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    res.body("removed");
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ProductFilter {
    price_range: Option<(Decimal, Decimal)>,
    stock: Option<u32>,
    unique: Option<bool>,
    keywords: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    excluded_keywords: Option<Vec<String>>,
    excluded_tags: Option<Vec<String>>,
    limit: Option<u32>,
    skip: Option<u32>,
}

impl ProductFilter {
    fn retrieve_products(
        &self,
    ) -> anyhow::Result<Vec<(String, (u32, u8, String, String, Vec<String>, Vec<String>))>> {
        let rx = DB.begin_read()?;
        let mut hm: HashMap<String, u32> = HashMap::new();
        let prs = rx.open_table(PRODUCTS)?;
        let mut skipped = 0;
        let mut res = vec![];
        if let Some(tags) = &self.tags {
            let tl = rx.open_multimap_table(TAG_LOOKUP)?;
            let mut layer = 0;
            let mut iter = tl.get(tags[layer].as_str())?;
            let mut n = iter.next();
            loop {
                if let Some(r) = n {
                    match r {
                        Ok(ag) => {
                            let pid = ag.value().to_string();
                            if let Some(k) = hm.get(&pid) {
                                hm.insert(pid, k + 1);
                            } else {
                                hm.insert(pid, 1);
                            }
                        }
                        Err(e) => {
                            println!("db product read error: {}", e);
                        }
                    }
                    n = iter.next();
                } else {
                    if tags.len() > layer {
                        layer += 1;
                        iter = tl.get(tags[layer].as_str())?;
                    } else {
                        break;
                    }
                }
            }
            for (name, k) in hm.iter() {
                if (*k as usize) != tags.len() {
                    continue;
                }
                let pid = name.as_str();
                if let Some(ag) = prs.get(pid)? {
                    let p = ag.value();
                    if filter_product(pid, &p, &self, true)? {
                        if self.skip.is_some_and(|s| s.gt(&skipped)) {
                            skipped += 1;
                        } else {
                            res.push((pid.to_string(), p));
                            if self.limit.is_some_and(|l| (l as usize) < res.len()) {
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            let mut iter = prs.iter()?;
            let mut n = iter.next();
            loop {
                if let Some(r) = n {
                    match r {
                        Ok(ag) => {
                            let p = ag.1.value();
                            let pid = ag.0.value();
                            if filter_product(pid, &p, &self, false)? {
                                if self.skip.is_some_and(|s| s.gt(&skipped)) {
                                    skipped += 1;
                                } else {
                                    res.push((pid.to_string(), p));
                                    if self.limit.is_some_and(|l| (l as usize) < res.len()) {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => println!("db product read error: {}", e),
                    }
                    n = iter.next(); //Err(anyhow!("didn't find any relevant products for this filter"))
                } else {
                    break;
                }
            }
        }
        Ok(res)
    }
}

fn filter_product(
    name: &str,
    p: &(u32, u8, String, String, Vec<String>, Vec<String>),
    pf: &ProductFilter,
    skip_tags: bool,
) -> anyhow::Result<bool> {
    if !skip_tags {
        if let Some(tags) = &pf.tags {
            for tag in tags {
                if !p.5.contains(tag) {
                    return Ok(false);
                }
            }
        }
    }
    if let Some(tags) = &pf.excluded_tags {
        for tag in tags {
            if p.5.contains(tag) {
                return Ok(false);
            }
        }
    }
    if let Some(kws) = &pf.keywords {
        for w in kws {
            if !name.contains(w) && !p.2.contains(w) {
                return Ok(false);
            }
        }
    }
    if let Some(kws) = &pf.excluded_keywords {
        for w in kws {
            if name.contains(w) && p.2.contains(w) {
                return Ok(false);
            }
        }
    }
    if let Some(u) = pf.unique {
        if !u && p.1 == 1 {
            return Ok(false);
        }
    }
    if let Some(s) = pf.stock {
        if p.0 < s {
            return Ok(false);
        }
    }
    if let Some((l, h)) = &pf.price_range {
        let d = Decimal::from_str_exact(&p.3)?;
        if l.gt(&d) || h.lt(&d) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[handler]
async fn fetch_products(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let mut pf = req.parse_body_with_max_size::<ProductFilter>(8196).await?;
    if pf.limit.is_none() {
        if req.cookie("auth").is_none() {
            pf.limit = Some(25);
        } else {
            let (_, is_admin) = auth_check_and_is_admin(req, res)?;
            if !is_admin {
                pf.limit = Some(1000);
            }
        }
    }
    let prs = pf.retrieve_products()?;
    res.headers_mut()
        .insert("Content-Type", "application/json".parse().unwrap());
    res.body(serde_json::to_string(&prs)?);
    Ok(())
}

#[handler]
async fn upload_files(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    req.set_secure_max_size(1024 * 1024 * 1024);
    if let Some(files) = req.files("files").await {
        let mut msgs = Vec::with_capacity(files.len());
        let mut sub_dir = "";
        for file in files {
            if let Some(file_name) = file.name() {
                if file_name.ends_with(".jpg") || file_name.ends_with(".png") {
                    sub_dir = "/products/images/"
                } else if file_name.ends_with(".css") {
                    sub_dir = "css/"
                } else if file_name.ends_with(".js") {
                    sub_dir = "js/"
                }
                let dest = format!("./assets/{}{}", sub_dir, file_name);
                println!("file {:?}, dest {}", file, dest);
                if let Err(e) = std::fs::copy(file.path(), Path::new(&dest)) {
                    res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                    res.render(Text::Plain(format!("file not found in request: {e}")));
                    return Err(anyhow!("file upload had issues"));
                } else {
                    msgs.push(dest);
                }
            }
        }
        res.render(Text::Plain(format!(
            "Files uploaded:\n\n{}",
            msgs.join("\n")
        )));
    } else {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Text::Plain(
            "file not found in request, may have been too large (100mb limit currently)",
        ));
        return Err(anyhow!("file upload didn't feature files?"));
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let local = !args.iter().any(|arg| arg == "-p");
    tracing_subscriber::fmt().init();

    let router = Router::new()
        .hoop(Compression::new().enable_zstd(CompressionLevel::Minsize))
        .push(Router::with_path("shop").get(shop_route))
        .push(Router::with_path("products").post(fetch_products))
        .push(
            Router::with_path("product")
                .post(product_insert)
                .push(Router::with_path("{name}").get(get_product))
                .push(Router::with_path("rm").post(product_remove)),
        )
        .push(
            Router::with_path("order")
                .push(Router::with_path("status").post(order_status_change))
                .push(Router::with_path("cancel").post(cancel_order))
                .push(Router::with_path("remove").post(order_remove))
                .post(place_order)
                .push(Router::with_path("{id}").get(view_order))
                .push(Router::with_path("tag").post(tag_order))
                .goal(view_orders),
        )
        .push(
            Router::with_path("register")
                .get(register)
                .delete(unregister),
        )
        .push(Router::with_path("signup").post(signup))
        .push(Router::with_path("signin").post(signin))
        .push(Router::with_path("signout").get(signout))
        .push(Router::with_path("mkadmin").post(make_admin)) //.push(Router::with_path("ws").goal(socket))
        .push(Router::with_path("upload").post(upload_files))//.push(Router::with_path("record/{this}").get(record))
        .push(Router::with_path("cmd").post(fire_command))
        .push(
            Router::with_path("memory").get(memory_page)
            .push(Router::with_path("{op}").goal(memory_api))
        )
        .push(Router::with_path("kuis").get(kop_huis))
        .push(Router::with_path("moot").post(mootroute))
        .push(
            Router::with_path("{*path}")
            .push(Router::with_path("textuality/{name}").post(save_textuality))
            .push(Router::with_path("textualities").get(list_textualities))
            .get(
                StaticDir::new(["assets"])
                    .include_dot_files(false)
                    .defaults("index.html"),
            ),
        );
    let _jh = expiry_thread();
    let mut service = Service::new(router);
    if !local {
        service = service.hoop(ForceHttps::new().https_port(80));
        let cert_data: Vec<u8> =
            fs::read("../certs/cert.pem").expect("put a valid cert file in ../certs as cert.pem");
        let key_data: Vec<u8> =
            fs::read("../certs/key.pem").expect("put a valid key file in ../certs as key.pem");
        let config = RustlsConfig::new(
            Keycert::new()
                .cert(cert_data.as_slice())
                .key(key_data.as_slice()),
        );
        let acceptor = TcpListener::new("0.0.0.0:80")
            .rustls(config)
            .join(TcpListener::new("0.0.0.0:443"))
            .bind()
            .await;
        Server::new(acceptor).serve(service).await;
    } else {
        let _d = watch_assets();
        let acceptor = TcpListener::new("0.0.0.0:4080").bind().await;
        Server::new(acceptor).serve(service).await;
    }
    EXPIRE_EXPIRY.store(1, std::sync::atomic::Ordering::Relaxed);
}

#[handler]
async fn fire_command(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let mut cmd: Command = Command::new(str::from_utf8(req.payload().await?)?);
    if let Some(l) = req.query::<&str>("l") { cmd.current_dir(l); }
    if let Some(args) = req.query::<&str>("args") { cmd.arg(args); }
    match cmd.spawn() {
        Ok(mut c) => {
            let says_what = if let Some(mut out) = c.stdout.take() {
                let mut reading = String::new();
                if let Err(e) = out.read_to_string(&mut reading) {
                    println!("cmd had nothing to say, but this error {}", e);
                }
                reading
            } else {
                "command ran".to_string()
            };
            res.render(Text::Plain(says_what));
        }
        Err(e) => {
            res.status_code(StatusCode::EXPECTATION_FAILED);
            res.render(Text::Plain(e.to_string()));
        }
    }
    Ok(())
}

static SEED_CACHE: OnceLock<[u8; SEED_BYTES]> = OnceLock::new();

fn establish_hashr(
    personalization: Option<&str>,
    personalization2: Option<&str>,
) -> anyhow::Result<sthash::Hasher, anyhow::Error> {
    let seed = if let Some(cached) = SEED_CACHE.get() {
        *cached
    } else {
        let seed: [u8; SEED_BYTES] = if let Ok(seed) = std::fs::read("./certs/fertalizer") {
            if let Some(a) = seed.as_array() {
                a.clone()
            } else {
                return Err(anyhow::anyhow!("corrupt fertalizer"));
            }
        } else {
            let mut seed = [0u8; SEED_BYTES];
            rand::rng().fill_bytes(&mut seed);
            std::fs::write("./certs/fertalizer", seed)?;
            seed
        };
        if let Err(e) = SEED_CACHE.set(seed) {
            println!("once lock is being poestlik, see: {:?}", e);
        }
        seed
    };
    Ok(Hasher::new(
        Key::from_seed(&seed, personalization.map(|p| p.as_bytes())),
        personalization2.map(|p| p.as_bytes()),
    ))
}

fn randstr(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

static EVENTCACHE: LazyLock<(broadcast::Sender<String>, broadcast::Receiver<String>)> =
    LazyLock::new(|| broadcast::channel(1024));

/*
#[handler]
async fn socket(req: &mut Request, res: &mut Response) -> anyhow::Result<(), StatusError> {
    if let Some(c) = req.cookie("auth") {
        match assay_claim(c.value().to_string()).await {
            Ok(c) => res.add_cookie(c),
            Err(e) => return Err(e),
        }
    } else {
        return Err(StatusError::bad_request());
    };

    WebSocketUpgrade::new()
        .upgrade(req, res, |mut ws| async move {
            let mut rx = EVENTCACHE.0.subscribe();
            loop {
                tokio::select! {
                    msg = ws.recv() => {
                        match msg {
                            Some(Ok(m)) => {
                                if let Err(e) = ws.send(m).await {
                                    println!("{:?}", e);
                                    return; // Client is weg
                                }
                            }
                            Some(Err(e)) => {
                                println!("{:?}", e);
                                return;
                            }
                            None => {},
                        }
                    }
                    evt = rx.recv() => {
                        match evt {
                            Ok(msg) => {
                                if let Err(e) = ws.send(Message::text(msg)).await {
                                    println!("{:?}", e);
                                    // return;
                                }
                            }
                            Err(e) => {
                                println!("fucked-up with {:?}", e);
                            },
                        }
                    }
                }
            }
        })
        .await
}*/

fn watch_assets() -> Debouncer<notify_debouncer_full::notify::INotifyWatcher, NoCache> {
    let mut debouncer = new_debouncer(
        std::time::Duration::from_millis(500),
        None,
        |result: DebounceEventResult| match result {
            Ok(events) => events.iter().for_each(|event| match event.kind {
                notify_debouncer_full::notify::EventKind::Modify(_) => handle_asset_change(event),
                notify_debouncer_full::notify::EventKind::Create(_) => handle_asset_change(event),
                notify_debouncer_full::notify::EventKind::Remove(_) => handle_asset_change(event),
                _ => {}
            }),
            Err(errors) => errors.iter().for_each(|error| println!("{error:?}")),
        },
    )
    .expect("couldn't get the file watcher even instantiated, not sure");
    debouncer
        .watch(
            "./assets",
            notify_debouncer_full::notify::RecursiveMode::Recursive,
        )
        .expect("watching the files is broken for some reason");
    return debouncer;
}

fn handle_asset_change(event: &DebouncedEvent) {
    for path in &event.paths {
        if let Some(file_name) = path.file_stem() {
            if let Some(ext) = path.extension() {
                let mut fine_mode = false;
                let mut in_views_otherwise = false; //const ZSTABLE: [&'static str; 4] = ["js", "css", "html", "svg"];
                let mut in_fine_views = false; /*if let Some(ext) = ext.to_str() { if ZSTABLE.contains(&ext) { if let Err(e) = zstd_dupe(path) { println!("failed to run zstd_dupe on {path:?} error: {e:?}"); } } }*/
                for c in path.components() {
                    let s = c.as_os_str();
                    if fine_mode {
                        if in_fine_views {
                            if let Some(pp) = path.parent() {
                                if let Ok(views) = get_filestems_of_ext(pp, "js") {
                                    let viewsjs =
                                        format!("var views = `{}`.split(' ')", views.join(" "));
                                    if let Some(the_path) = pp.parent() {
                                        if let Err(e) = fs::write(
                                            Path::new(the_path).join("reserve/views.js"),
                                            viewsjs,
                                        ) {
                                            println!("{e:?}")
                                        }
                                    }
                                }
                            }
                        } else if s == "views" {
                            in_fine_views = true;
                        }
                    } else if s == "fine" {
                        fine_mode = true;
                    } else if s == "views" {
                        in_views_otherwise = true;
                    } else {
                        if in_views_otherwise {
                            if let Some(pp) = path.parent() {
                                if let Ok(views) = get_filestems_of_ext(pp, "js") {
                                    let viewsjs =
                                        format!("var views = `{}`.split(' ')", views.join(" "));
                                    if let Some(the_path) = pp.parent() {
                                        if let Err(e) = fs::write(
                                            Path::new(the_path).join("reserve/views.js"),
                                            viewsjs,
                                        ) {
                                            println!("{e:?}");
                                        } else {
                                            if let Err(e) = EVENTCACHE.0.send(views.join(" ")) {
                                                println!("{e:?}");
                                            }
                                        }
                                        return;
                                    }
                                }
                            }
                        }
                    }
                } // todo make .zst compressed versions and set up the server to serve them
                println!(
                    "changed file {}.{}",
                    file_name.to_string_lossy(),
                    ext.to_string_lossy()
                );
            } else {
                println!("changed dir? {path:?}");
            }
        } else {
            println!("changed dir? {path:?}");
        }
    }
}

fn get_filestems_of_ext<P: AsRef<Path>>(dir: P, ext: &str) -> std::io::Result<Vec<String>> {
    let entries = fs::read_dir(dir)?;
    let file_stems = entries
        .filter_map(|res| res.ok())
        .map(|e| e.path())
        .filter(|path| path.is_file() && path.extension().and_then(|s| s.to_str()) == Some(ext))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    Ok(file_stems)
}

#[handler]
async fn shop_route(_req: &mut Request, _res: &mut Response) -> salvo::Result<NamedFile> {
    NamedFile::open("./assets/shop.html").await
}

#[handler]
async fn memory_page(_req: &mut Request, _res: &mut Response) -> salvo::Result<NamedFile> {
    NamedFile::open("./assets/memory.html").await
}

#[handler]
async fn kop_huis(_req: &mut Request, _res: &mut Response) -> salvo::Result<NamedFile> {
    NamedFile::open("./assets/kuis.html").await
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn gematria_dual_sum(data: &[u8]) -> (u64, u64) {
    unsafe {
        let len  = data.len();
        let base = data.as_ptr();
        let mut i = 0usize;

        // ── Constants ────────────────────────────────────────────────────────
        let lower_mask  = _mm256_set1_epi8(0x20u8 as i8);     // OR  → fold case
        let sub_offset  = _mm256_set1_epi8((b'a' - 1) as i8); // 96: a→1 … z→26
        let max_val     = _mm256_set1_epi8(26);               // upper clamp
        let twentyseven = _mm256_set1_epi8(27);               // rev = 27 − std
        let zero        = _mm256_setzero_si256();

        // 4 chains × 2 outputs = 8 accumulators. Leaves ~8 YMMs for the working
        // set, so nothing spills on x86_64's 16-register file.
        let mut s0 = zero; let mut s1 = zero; let mut s2 = zero; let mut s3 = zero;
        let mut r0 = zero; let mut r1 = zero; let mut r2 = zero; let mut r3 = zero;

        // ── Per-32-byte kernel ───────────────────────────────────────────────
        macro_rules! block {
            ($sa:ident, $ra:ident, $off:expr) => {{
                let v       = _mm256_loadu_si256(base.add($off) as *const __m256i);
                let lc      = _mm256_or_si256(v, lower_mask);      // fold case
                let sub     = _mm256_sub_epi8(lc, sub_offset);     // 'a'→1 … 'z'→26
                let clamped = _mm256_min_epu8(sub, max_val);       // letters untouched
                // mask = (sub <= 26) AND (sub >= 1).
                //   - cmpeq(sub, min(sub, 26)) is true for sub ∈ [0, 26]
                //   - cmpgt(sub, 0) removes sub == 0, which is where '@' (0x40)
                //     and '`' (0x60) alias after `| 0x20` — the original bug.
                let in_rng  = _mm256_cmpeq_epi8(sub, clamped);
                let nonzero = _mm256_cmpgt_epi8(sub, zero);
                let mask    = _mm256_and_si256(in_rng, nonzero);

                let gval = _mm256_and_si256(sub, mask);            // 0 for non-letters
                let rval = _mm256_and_si256(
                    _mm256_sub_epi8(twentyseven, gval),
                    mask,
                );

                $sa = _mm256_add_epi64($sa, _mm256_sad_epu8(gval, zero));
                $ra = _mm256_add_epi64($ra, _mm256_sad_epu8(rval, zero));
            }};
        }

        // ── 4× unrolled main loop (128 B / iter) ─────────────────────────────
        while len - i >= 128 {
            block!(s0, r0, i);
            block!(s1, r1, i +  32);
            block!(s2, r2, i +  64);
            block!(s3, r3, i +  96);
            i += 128;
        }
        // ── Remaining whole 32-byte blocks ───────────────────────────────────
        while len - i >= 32 {
            block!(s0, r0, i);
            i += 32;
        }

        // ── Tree-reduce 4 YMMs → one u64 ─────────────────────────────────────
        macro_rules! hsum4 {
            ($a:expr, $b:expr, $c:expr, $d:expr) => {{
                let t0 = _mm256_add_epi64($a, $b);
                let t1 = _mm256_add_epi64($c, $d);
                let t  = _mm256_add_epi64(t0, t1);
                let lo = _mm256_castsi256_si128(t);
                let hi = _mm256_extracti128_si256(t, 1);
                let s  = _mm_add_epi64(lo, hi);
                (_mm_cvtsi128_si64(s) as u64)
                    .wrapping_add(_mm_extract_epi64(s, 1) as u64)
            }};
        }
        let mut std_sum = hsum4!(s0, s1, s2, s3);
        let mut rev_sum = hsum4!(r0, r1, r2, r3);

        // ── Scalar tail (< 32 bytes) ─────────────────────────────────────────
        while i < len {
            let b = *base.add(i);
            i += 1;
            let v = if b.is_ascii_uppercase() {
                (b - b'A' + 1) as u64
            } else if b.is_ascii_lowercase() {
                (b - b'a' + 1) as u64
            } else {
                continue;
            };
            std_sum += v;
            rev_sum += 27 - v;
        }

        (std_sum, rev_sum)
    }
}

const STATE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state");
const TO_REUSE: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("reusables");
const MEMORY_INDEX: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("memory index");
const MEMORIES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("memories");

/* idea is that length pointers can be avoided by sharing the index into sized-brackets
const MEMORIES_1B: TableDefinition<u8, &[u8]> = TableDefinition::new("memories 1B");
const MEMORIES_2B: TableDefinition<u16, &[u8]> = TableDefinition::new("memories 2B");
const MEMORIES_3B: TableDefinition<(u8, u8, u8), &[u8]> = TableDefinition::new("memories 3B");
const MEMORIES_4B: TableDefinition<u32, &[u8]> = TableDefinition::new("memories 4B");
*/
// const DEDUP: TableDefinition<&[u8], &[u8]> = TableDefinition::new("dedup"); idk yet can do client side carefuleness too (still)
// const PREFIXED: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("prefixed:");
// const SUFIXED: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("sufixed:");

const MEMORY_GEMATRIA: MultimapTableDefinition<u32, &[u8]> = MultimapTableDefinition::new("memg");
const MEMORY_GEMATRIA_REVERSED: MultimapTableDefinition<u32, &[u8]> = MultimapTableDefinition::new("memg reversed");
const ALLOCATED: &'static [u8] = b"allocated".as_slice();
const LISTS: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("lists");
const NOTES: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("notes");

const NOTED_ON: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("noted on");

fn add_to_list(ln: &[u8], v: &[u8])  -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut lists = wx.open_multimap_table(LISTS)?;
        lists.insert(ln, v)?;
    }
    wx.commit()?;
    Ok(())
}

fn get_list(ln: &[u8])  -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let rx = MEMORY.begin_read()?;
    let mut mmv = rx.open_multimap_table(LISTS)?.get(ln)?;
    let memories = rx.open_table(MEMORIES)?;
    let mut hm = HashMap::new();
    while let Some(r) = mmv.next() {
        match r {
            Ok(ag) => {
                let k = ag.value();
                if let Some(ag) = memories.get(k)? {
                    hm.insert(k.to_vec(), ag.value().to_vec());
                }
            },
            Err(e) => {
                println!("storage error during get_notes_on: {} {:?}", e, ln);
            }
        }
    }
    Ok(hm)
}
/*
fn rm_list(ln: &[u8])  -> anyhow::Result<Vec<Vec<u8>>> {
    let wx = MEMORY.begin_write()?;
    let mut vc = vec![];
    {
        let mut lists = wx.open_multimap_table(LISTS)?;
        while let Some(r) = lists.remove_all(ln)?.next() {
            match r {
                Ok(ag) => {
                    vc.push(ag.value().to_vec());
                },
                Err(e) => {
                    println!("storage error during get_notes_on: {} {:?}", e, ln);
                }
            }
        }
    }
    wx.commit()?;
    Ok(vc)
}*/

fn rm_from_list(ln: &[u8], v: &[u8])  -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut lists = wx.open_multimap_table(LISTS)?;
        lists.remove(ln, v)?;
    }
    wx.commit()?;
    Ok(())
}

fn note_on(note: &[u8], on: &[u8]) -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        notes.insert(on, note)?;
        noted_on.insert(note, on)?;
    }
    wx.commit()?;
    Ok(())
}

fn get_notes_on(this: &[u8]) -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let mut found: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let rx = MEMORY.begin_read()?;
    let notes = rx.open_multimap_table(NOTES)?;
    let memories = rx.open_table(MEMORIES)?;
    while let Some(r) = notes.get(this)?.next() {
        match r {
            Ok(ag) => {
                let k = ag.value();
                if let Some(ag) = memories.get(k)? {
                    found.insert(k.to_vec(), ag.value().to_vec());
                } // todo cleanup
            },
            Err(e) => {
                println!("storage error during get_notes_on: {} {:?}", e, this);
            }
        }
    }
    if found.len() == 0 {
        return Err(anyhow!("didn't find any notes with that"));
    }
    Ok(found)
}

fn notes_on(hm: HashMap<&[u8], &[u8]>) -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        for (on, note) in hm { 
            notes.insert(on, note)?;
            noted_on.insert(note, on)?;
        }
    }
    wx.commit()?;
    Ok(())
}

fn rm_note(note: &[u8], on: &[u8]) -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        notes.remove(on, note)?;
        noted_on.remove(note, on)?;
    }
    wx.commit()?;
    Ok(())
}

/*
fn rm_notes(on: &[u8]) -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        while let Some(r) = notes.remove_all(on)?.next() {
            match r {
                Ok(ag) => {
                    noted_on.remove(ag.value(), on)?;
                },
                Err(e) => {
                    println!("storage error during rm_notes: {}", e);
                }
            }
        }
    }
    wx.commit()?;
    Ok(())
}*/

fn init_state(db: &mut Database) -> anyhow::Result<bool> {
    Ok(db.compact()?)
}

static MEMORY: LazyLock<Database> = LazyLock::new(|| {
    let mut db = Database::create("./mem").expect("opening/creating db had issues");
    if init_state(&mut db).expect("state init failed") { println!("memory db compacted"); }
    db
});

pub fn tokenate(data: &[u8]) -> Vec<&[u8]> {
    let mut tokens = Vec::new();
    let mut word_start: Option<usize> = None;
    let chunk_size = 32;
    let len = data.len();
    let mut i = 0;
    let spaces = u8x32::splat(b' ');
    let newlines = u8x32::splat(b'\n');
    let tabs = u8x32::splat(b'\t');
    let returns = u8x32::splat(b'\r');
    while i + chunk_size <= len {
        let chunk = u8x32::from_slice(&data[i..i + chunk_size]);
        let is_delimit = chunk.simd_eq(spaces) | chunk.simd_eq(newlines) | chunk.simd_eq(tabs) | chunk.simd_eq(returns);
        let mut mask = (!is_delimit).to_bitmask();
        while mask != 0 {
            let bit = mask.trailing_zeros() as usize;
            let current_idx = i + bit;
            if word_start.is_none() { word_start = Some(current_idx); } 
            let search_mask = !mask & !((1 << bit) - 1);
            if search_mask != 0 {
                let next_delimiter_bit = search_mask.trailing_zeros() as usize;
                let end = i + next_delimiter_bit;
                if let Some(start) = word_start.take() { tokens.push(&data[start..end]); } 
                mask &= !((1 << next_delimiter_bit) - 1);
            } else { mask = 0; }
        }
        i += chunk_size;
    }
    for j in i..len {
        let b = data[j];
        let is_delimit = b == b' ' || b == b'\n' || b == b'\t' || b == b'\r';
        if is_delimit {
            if let Some(start) = word_start.take() { tokens.push(&data[start..j]); }
        } else if word_start.is_none() { word_start = Some(j); }
    }
    if let Some(start) = word_start { tokens.push(&data[start..len]); }
    tokens.dedup();
    tokens
}

fn saturate(quidity: &[u8], mut t: Option<Vec<u8>>) -> anyhow::Result<Vec<u8>> {
    let mb_jh = if let Ok(content) = String::from_utf8(quidity.to_vec()) {
        let jh = std::thread::spawn(move || -> Vec<Vec<f32>> {
            if content.ends_with(".png") || content.ends_with(".jpg") || content.ends_with(".webp") {
                match EMBEDDER.embed_images(&[content]) {
                    Ok(emb)  => emb,
                    Err(e) => {
                        println!("embedder blew up during image run, {}", e);
                        vec![]
                    }
                }
            } else {
                match EMBEDDER.embed_texts(&[content]) {
                    Ok(emb)  => emb,
                    Err(e) => {
                        println!("text embedding run blew up the embedder thread, {}", e);
                        vec![]
                    }
                }
            }
        });
        Some(jh)
    } else { None };
    let wx = MEMORY.begin_write()?;
    {
        let mut state = wx.open_table(STATE)?;
        let mut reusables = wx.open_multimap_table(TO_REUSE)?;
        let mut status: u8 = 0;
        if t.is_none() {
            let mut iter = reusables.get(ALLOCATED)?;
            if let Some(r) = iter.next() {
                t = Some(r?.value().to_vec());
                status = 1;
            } else if let Some(mut b) = state.get(ALLOCATED)?.map(|ag| ag.value().to_vec()) {
                let l = b.len();
                t = Some(if l == 4 {
                    (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) + 1).to_be_bytes().to_vec()
                } else if l == 3 {
                    if b[2] == 255 {
                        if b[1] == 255 {
                            if b[0] == 255 {
                                b = vec![0,0,0,0];
                            } else {
                                b[0] += 1;
                                b[1] = 0;
                                b[2] = 0;
                            }
                        } else {
                            b[1] += 1;
                            b[2] = 0;
                        }
                    } else {
                        b[2] += 1;
                    }
                    b
                } else if l == 2 {
                    let at = u16::from_be_bytes([b[0], b[1]]);
                    if at == u16::MAX { vec![0,0,0] }
                    else { (at + 1).to_be_bytes().to_vec() }
                } else if l == 1 {
                    if b[0] == u8::MAX { vec![0,0] } else { vec![b[0] + 1] }
                } else {
                    return Err(anyhow!("weird sized allocated, not normal"));
                });
                status = 2;
            } else {
                t = Some(vec![0]);
            }
        }
        if let Some(this) = &t {
            let mut mem = wx.open_table(MEMORIES)?;
            let mut embeddings = wx.open_table(EMBEDDINGS)?;
            let mut memg = wx.open_multimap_table(MEMORY_GEMATRIA)?;
            let mut memgr = wx.open_multimap_table(MEMORY_GEMATRIA_REVERSED)?;
            let mut memi = wx.open_multimap_table(MEMORY_INDEX)?;
            let tas = this.as_slice();
            mem.insert(tas, quidity)?;
            unsafe { 
                let (f, r) = gematria_dual_sum(quidity);
                memg.insert(f as u32, tas)?;
                memgr.insert(r as u32, tas)?;
            }
            for tk in tokenate(quidity) { memi.insert(tk, tas)?; }
            if status == 1 {
                reusables.remove(ALLOCATED, tas)?;
            } else if status == 2 || (tas.len() == 1 && tas[0] == 0) {
                state.insert(ALLOCATED, tas)?;
            }
            if let Some(Ok(mut embs)) = mb_jh.map(|jh| jh.join()) {
                if let Some(embedding) = embs.pop() {
                    embeddings.insert(tas, embedding)?;
                } else {
                    return Err(anyhow!("embedding not generated for some reason"));
                }
            }
        }
    }
    if let Some(this) = t {
        wx.commit()?;
        Ok(this)
    } else {
        wx.abort()?;
        Err(anyhow!("fucked up mid way with saturating this memory"))
    }
}

fn remember(this: &[u8]) -> anyhow::Result<Vec<u8>> {
    if let Some(ag) = MEMORY.begin_read()?.open_table(MEMORIES)?.get(this)? {
        Ok(ag.value().to_vec())
    } else {
        Err(anyhow!("didn't find that"))
    }
}

fn backup_memories_to_file() -> anyhow::Result<u64> {
    let rx = MEMORY.begin_read()?;
    let mem = rx.open_table(MEMORIES)?;
    let embeddings = rx.open_table(EMBEDDINGS)?;
    let mut all_of_it: Vec<(String, Vec<u8>)> = Vec::with_capacity(mem.len()? as usize);
    let mut berrings: Vec<(Vec<u8>, Vec<f32>)> = Vec::with_capacity(mem.len()? as usize);
    let mut binary_otherwize: Vec<Vec<u8>> = vec![];
    let mut iter = mem.iter()?;
    while let Some(r) = iter.next() {
        match r {
            Ok((k_ag, v_ag)) => {
                let v = v_ag.value();
                if let Ok(stwing) = str::from_utf8(v) {
                    let id = k_ag.value();
                    all_of_it.push((stwing.to_string(), id.to_vec()));
                    if let Some(ag) = embeddings.get(id)? {
                        berrings.push((id.to_vec(), ag.value()));
                    }
                } else {
                    binary_otherwize.push(v.to_vec());
                }
            }, 
            Err(e) => {
                println!("daar is 'n gepoest, {}", e);
            }
        }
    }
    std::fs::write("./all-of-it.json", serde_json::to_string(&all_of_it)?)?;
    std::fs::write("./all-of-the-embeddings-with-ids.json", serde_json::to_string(&berrings)?)?;
    std::fs::write("./all-of-it-binaries.json", serde_json::to_string(&binary_otherwize)?)?;
    Ok(mem.len()?)
} /*fn memory_search(query: &[u8]) -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> { let tkns = tokenate(query)?; let rx = MEMORY.begin_read()?; let mi = rx.open_multimap_table(MEMORY_INDEX)?; let mem = rx.open_table(MEMORIES)?; let mut mmv = mi.get(tkns[0].as_slice())?; let mut out = HashMap::new(); loop { if let Some(r) = mmv.next() { let ag = r?; let k = ag.value(); if let Some(ag) = mem.get(k)? { let v = ag.value(); if tokenate(v)?.par_iter().any(|tkn| tkns.contains(&tkn)) { out.insert(k.to_vec(), v.to_vec()); } } } else { break; } } if out.len() == 0 { return Err(anyhow!("didn't find anything for that query")); } Ok(out) }*/

fn memory_search(query: &[u8], also_saturate: bool) -> anyhow::Result<(HashMap<Vec<u8>, Vec<u8>>, Option<Vec<u8>>)> {
    let saturated = if also_saturate {
        if let Ok((fnd, _)) = memory_search(query, false) {
            if fnd.iter().find(|t| query.eq(t.1)).is_none() {
                Some(saturate(query, None)?)
            } else {
                None
            }
        } else {
            Some(saturate(query, None)?)
        }
    } else {
        None
    };
    let tkns = tokenate(query);
    let threshold = ((tkns.len() as f32 * 0.75).ceil() as usize).max(1);
    let mut scores: HashMap<Vec<u8>, usize> = HashMap::new();
    let (tx, rc) = mpsc::channel::<Vec<u8>>();
    let rx= MEMORY.begin_read()?;
    tkns.par_iter().for_each(|tkn| {
        let tx = tx.clone();
        if let Ok(mi) = rx.open_multimap_table(MEMORY_INDEX) {
                if let Ok(mut mmv) = mi.get(tkn) {
                    loop {
                        match mmv.next() {
                            None => break,
                            Some(Ok(ag)) => { let _ = tx.send(ag.value().to_vec()); }
                            Some(Err(e)) => {
                                println!("search oof'itty qua: {}", e);
                                break;
                            },
                        }
                    }
                }
            }
    });
    drop(tx);
    for k in rc { *scores.entry(k).or_insert(0) += 1; }
    let mut out = HashMap::new();
    let candidates: Vec<Vec<u8>> = scores.into_iter().filter(|(_, hits)| *hits >= threshold).map(|(k, _)| k).collect();
    let (tx, rc) = mpsc::channel::<(Vec<u8>, Vec<u8>)>();
    candidates.par_iter().for_each(|k| {
        let tx = tx.clone();
        if let Ok(mem) = rx.open_table(MEMORIES) {
            if let Ok(Some(ag)) = mem.get(k.as_slice()) {
                if let Err(e) = tx.send((k.clone(), ag.value().to_vec())) {
                    println!("search oof'itty qua: {}", e);
                }
            }
        }
    });
    drop(tx);
    for (k, v) in rc { out.insert(k, v); }
    if out.is_empty() { 
        return Err(if saturated.is_none() {
            anyhow!("didn't find anything for that query")
        } else {
            anyhow!("didn't find anything for that query, but it was saturated")
        });
    }
    Ok((out, saturated))
}

fn forget(this: &[u8]) -> anyhow::Result<Vec<u8>> {
    let wx = MEMORY.begin_write()?;
    if let Some(o) = {
        let old = wx.open_table(MEMORIES)?.remove(this)?.map(|ag| ag.value().to_vec());
        if let Some(quidity) = &old {
            wx.open_multimap_table(TO_REUSE)?.insert(ALLOCATED, this)?;
            let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
            while let Some(r) = wx.open_multimap_table(NOTES)?.remove_all(this)?.next() {
                match r {
                    Ok(ag) => {
                        noted_on.remove(ag.value(), this)?;
                    },
                    Err(e) => {
                        println!("storage error during rm_notes: {}", e);
                    }
                }
            }
            let tkns = tokenate(quidity.as_slice());
            let mut memg = wx.open_multimap_table(MEMORY_GEMATRIA)?;
            let mut memgr = wx.open_multimap_table(MEMORY_GEMATRIA_REVERSED)?;
            unsafe { 
                let (f, r) = gematria_dual_sum(quidity.as_slice());
                memg.remove(f as u32, this)?;
                memgr.remove(r as u32, this)?;
            }
            let mut memi = wx.open_multimap_table(MEMORY_INDEX)?;
            for tk in tkns { memi.remove(tk, this)?; }
            wx.open_table(EMBEDDINGS)?.remove(this)?;
        }
        old
    } {
        wx.commit()?;
        Ok(o)
    } else {
        wx.abort()?;
        Err(anyhow!("there was nothing with that name anyways"))
    }
}

fn gsearch(sum: u32, rev: bool) -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let rx = MEMORY.begin_read()?;
    let mem = rx.open_table(MEMORIES)?;
    let memg = rx.open_multimap_table(if rev { MEMORY_GEMATRIA_REVERSED } else { MEMORY_GEMATRIA })?;
    let mut mmv = memg.get(sum)?;
    let mut out = HashMap::new();
    loop {
        match mmv.next() {
            Some(Ok(ag)) => {
                let k = ag.value();
                if let Some(ag) = mem.get(k)? {
                    out.insert(k.to_vec(), ag.value().to_vec());
                }
            },
            Some(Err(e)) => println!("storage error in the memory in redb: {}", e),
            None => break,
        }
    }
    if out.is_empty() { return Err(anyhow!("didn't find anything for that number gsearch wize")); }
    Ok(out)
}
struct FWD {}
impl FWD {
    #[inline]
    fn server(res: &mut Response, e: anyhow::Error) {
        FWD::default(res, e, StatusCode::INTERNAL_SERVER_ERROR);
    }
    #[inline]
    fn not_found(res: &mut Response, e: anyhow::Error) {
        FWD::default(res, e, StatusCode::NOT_FOUND);
    }
    #[inline]
    fn bad_req(res: &mut Response, e: anyhow::Error) {
        FWD::default(res, e, StatusCode::BAD_REQUEST);
    }
    #[inline]
    fn default(res: &mut Response, e: anyhow::Error, sc: StatusCode) {
        res.status_code(sc);
        res.render(Text::Plain(e.to_string()));
    }
}

fn serialize_list_of_lists(data: &Vec<Vec<u8>>) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&(data.len() as u32).to_be_bytes());
    for inner_vec in data {
        let len = inner_vec.len() as u32;
        buffer.extend_from_slice(&len.to_be_bytes());
        buffer.extend_from_slice(inner_vec);
    }
    buffer
}

#[inline]
fn handle_fore_byted_bytes_pair(bytes: &mut Vec<u8>) -> anyhow::Result<(&[u8], &[u8])> {
    if bytes.len() >= 3 {
        let fb = bytes.remove(0) as usize;
        return Ok(bytes.split_at(fb));
    }
    Err(anyhow!("that's a bad split"))
}

#[handler]
async fn memory_api(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let mut status: u8 = 0;
    if let Some(op) = req.param("op") {
        match op {
            "remember" => {},
            "saturate" => { status = 1; },
            "forget" => { status = 2; },
            "search" => { status = 3; }
            "gsearch" => { status = 4; }
            "note" => { status = 5; }
            "notes" => { status = 6; }
            "list" => { status = 7; }
            "enlist" => { status = 8; }
            "delist" => { status = 9; }
            "unlist" => { status = 10; }
            "count" => { status = 11; }
            "esearch" => { status = 12; }
            "backup" => { status = 13; }
            _ => {
                FWD::bad_req(res, anyhow!("op snafu"));
                return Ok(());
            },
        }
    } else {
        FWD::bad_req(res, anyhow!("u wot m8?"));
        return Ok(());
    }

    if status == 0 {
        match remember(req.payload().await?) {
            Ok(b) => { res.body(b); },
            Err(e) => {
                FWD::not_found(res, e);
                return Ok(());
            }
        }
    } else if status == 1 {
        let id: Option<Vec<u8>> = req.query::<&str>("k")
            .map(|k| k.split(",")
            .filter_map(|b| if let Ok(b) = u8::from_str(b) { Some(b) } else { None })
            .collect::<Vec<u8>>());
        let dedup = req.query::<u8>("n").is_none();
        let payload = req.payload().await?;
        if dedup && id.is_none() {
            match memory_search(payload, true) {
                Ok((fnd, okey)) => if let Some(key) = okey {
                    res.body(key);
                } else if let Some((key, _)) = fnd.into_iter().find(|(_k, v)| v.eq(payload)) {
                    res.body(key);
                } else {
                    FWD::server(res, anyhow!("things didn't saturate properly for some reason"));
                }
                Err(e) => FWD::server(res, e)
            }
            return Ok(());
        } else {
            res.body(saturate(payload, id)?); // todo batch insert
        }
    } else if status == 2 {
        res.body(forget(req.payload().await?)?); // todo batch forget
    } else if status == 3 || status == 4 || status == 6 || status == 7 || status == 12 {
        let r = if status == 4 {
            gsearch(req.parse_body().await?, req.uri().query().is_some_and(|q| q.starts_with("r")))
        } else if status == 6 {
            get_notes_on(req.payload().await?)
        } else if status == 7 {
            get_list(req.payload().await?)
        } else if status == 12 {
            let sensitivity = req.query("s").unwrap_or_else(|| 0.8);
            let also_saturate = req.query::<u8>("m").is_some_and(|q| q == 1);
            let limit = req.query("l").unwrap_or_else(|| 100000);
            search_embeddings(str::from_utf8(req.payload_with_max_size(49000000000).await?)?, sensitivity, limit, also_saturate)
        } else {
            let also_saturate = req.query::<u8>("m").is_some_and(|q| q == 1);
            let r = memory_search(req.payload_with_max_size(99000000).await?, also_saturate);
            match r {
                Ok((fnd, saturated_key)) => {
                    if let Some(sk) = saturated_key {
                        res.headers_mut().insert(
                            "X-Saturated-Key", 
                            format!("{}", sk.into_iter().map(|b| b.to_string()).collect::<Vec<String>>().join(",")).parse().unwrap()
                        );
                    }
                    Ok(fnd)
                },
                Err(e) => Err(e),
            }
        };
        if let Err(e) = r {
            FWD::not_found(res, e);
            return Ok(()); // without returning ok, salvo will send a whole html error thing without any meaningful details, so.. workaround is manually filling it with an error and setting the code to that
        }
        let m = r.unwrap();
        // imagine saying instead of key_len -> key -> value_len -> value -> repeat:
        // key length bracket, number of keys -> keys of that length -> number of values, ...in-key's-order(value_len -> value) -> repeat
        if m.len() == 0 { FWD::not_found(res, anyhow!("no results for a list of those details")); } //let mut buffer = Vec::with_capacity(hm.len() * 2); for (key, value) in hm { buffer.write_all(&(key.len() as u32).to_be_bytes())?; buffer.write_all(&key)?; buffer.write_all(&(value.len() as u32).to_be_bytes())?; buffer.write_all(&value)?; }
        res.headers_mut().insert("content-type", "application/json".parse().unwrap());
        res.body(kraal(&m)?);
    } else if status == 5 {
        let mut pl = req.payload().await?.to_vec();
        let (note, on) = handle_fore_byted_bytes_pair(&mut pl)?;
        if req.uri().query().is_some_and(|q| q == "d") {
            match rm_note(note, on) {
                Ok(()) => { res.status_code(StatusCode::ACCEPTED); },
                Err(e) => { FWD::server(res, e); }
            }
        } else if let Err(e) = note_on(note, on) {
            FWD::server(res, e);
        } else {
            res.status_code(StatusCode::ACCEPTED);
        }
        return Ok(());
    } else if status == 8 {
        let mut pl = req.payload().await?.to_vec();
        let (ln, v) = handle_fore_byted_bytes_pair(&mut pl)?;
        println!("{:?} : {:?}", ln, v);
        add_to_list(ln, v)?;
        res.status_code(StatusCode::ACCEPTED);
        return Ok(());
    } else if status == 9 {
        let mut pl = req.payload().await?.to_vec();
        let (ln, v) = handle_fore_byted_bytes_pair(&mut pl)?;
        rm_from_list(ln, v)?;
        res.status_code(StatusCode::ACCEPTED);
        return Ok(());
    } else if status == 10 {
        res.body(serialize_list_of_lists(&(rm_list(req.payload().await?)?)));
        res.status_code(StatusCode::ACCEPTED);
        return Ok(());
    } else if status == 11 {
        let count = MEMORY.begin_read()?.open_table(MEMORIES)?.len()?;
        res.render(Text::Plain(count.to_string()));
    } else if status == 13 {
        let count = backup_memories_to_file()?;
        res.status_code(StatusCode::ACCEPTED);
        res.render(Text::Plain(format!("it was writt'n ({})", count)));
        return Ok(());
    }
    res.status_code(StatusCode::OK);
    Ok(())
}

/*fn products_by_price_range(l: &str, h: &str) -> anyhow::Result<Vec<String>> {
    let rx = DB.begin_read()?;
    let mut res = vec![];
    let mut iter = rx.open_multimap_table(PRICE_LOOKUP)?.range(l..h)?;
    let mut n = iter.next();
    loop {
        match &n {
            Some(o) => match o {
                Ok((ag, _)) => res.push(ag.value().to_string()),
                Err(e) => return Err(anyhow!("{}", e.to_string())),
            },
            None => break,
        }
        n = iter.next();
    }
    Ok(res)
}
fn rm_session(sid_hashed: Vec<u8>) -> anyhow::Result<()> {
    let wtx = DB.begin_write()?;
    {
        let mut sess = wtx.open_table(SESSIONS)?;
        if let Some(ag) = sess.remove(sid_hashed.as_slice())? {
            let s = ag.value();
            wtx.open_multimap_table(SESSIONS_LOOKUP)?
                .remove(s.0, sid_hashed.as_slice())?;
            wtx.open_table(EXPIRIES_SESSIONS)?.remove(s.1)?;
        }
    }
    wtx.commit()?;
    Ok(())
} // static SESSIONCACHE: LazyLock<Cache<Vec<u8>, >> = LazyLock::new(|| Cache::new(40000)); */

pub fn from_cents(cents: u64) -> String { // from cents translates a u64 (cents) to a string price representation
    (Decimal::from(cents) / dec!(100)).round_dp(2).to_string()
}
pub fn to_cents(price_str: &str) -> anyhow::Result<u64> { // to_cents translates a price &str to u64 as a cent representation
    let cents = Decimal::from_str(price_str)?.round_dp(2) * dec!(100);
    cents
        .to_u64()
        .ok_or_else(|| anyhow!("converting the price ran into issues"))
}/*fn dot_product(a: &[u64], b: &[u64]) -> u64 { a.iter().zip(b.iter()).map(|(x, y)| x * y).sum() }*/

fn rle_encode(lengths: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let val = lengths[i];
        let mut run = 1u8;
        while i + (run as usize) < lengths.len()
            && lengths[i + run as usize] == val
            && run < 255
        { run += 1; }
        out.push(run);
        out.extend_from_slice(&val.to_le_bytes());
        i += run as usize;
    }
    out
}

fn rle_decode(data: &[u8], count: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(count);
    let mut curr = 0;
    while out.len() < count && curr + 2 < data.len() {
        let run = data[curr] as usize;
        let val = u16::from_le_bytes([data[curr+1], data[curr+2]]);
        curr += 3;
        for _ in 0..run { if out.len() < count { out.push(val); } }
    }
    out
}

pub fn kraal(map: &HashMap<Vec<u8>, Vec<u8>>) -> anyhow::Result<Vec<u8>> {
    let n = map.len() as u32;
    let mut key_lens = Vec::with_capacity(map.len());
    let mut val_lens = Vec::with_capacity(map.len());
    let mut key_blob = Vec::new();
    let mut val_blob = Vec::new();

    for (k, v) in map {
        key_lens.push(k.len() as u16);
        val_lens.push(v.len() as u16);
        key_blob.extend_from_slice(k);
        val_blob.extend_from_slice(v);
    }

    let kdir = rle_encode(&key_lens);
    let vdir = rle_encode(&val_lens);
    
    let mut out = Vec::new();
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(key_blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&(val_blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&(kdir.len() as u32).to_le_bytes());
    out.extend_from_slice(&kdir);
    out.extend_from_slice(&key_blob);
    out.extend_from_slice(&(vdir.len() as u32).to_le_bytes());
    out.extend_from_slice(&vdir);
    out.extend_from_slice(&val_blob);
    Ok(out)
}

const KDIR_START: usize = 16;

pub fn ontkraal(data: &[u8]) -> anyhow::Result<HashMap<Vec<u8>, Vec<u8>>> {
    let n = u32::from_le_bytes(data[0..4].try_into()?) as usize;
    let keys_blob_len = u32::from_le_bytes(data[4..8].try_into()?) as usize;
    let kdir_len = u32::from_le_bytes(data[12..16].try_into()?) as usize;

    let key_blob_start = KDIR_START + kdir_len;
    let key_lens = rle_decode(&data[KDIR_START..key_blob_start], n);
    
    let vdir_len_offset = key_blob_start + keys_blob_len;
    let vdir_len = u32::from_le_bytes(data[vdir_len_offset..vdir_len_offset+4].try_into()?) as usize;
    let vdir_start = vdir_len_offset + 4;
    let val_blob_start = vdir_start + vdir_len;
    let val_lens = rle_decode(&data[vdir_start..val_blob_start], n);

    let mut kp = key_blob_start;
    let mut vp = val_blob_start;
    let mut mp = HashMap::new();
    for i in 0..n {
        let kl = key_lens[i] as usize;
        let vl = val_lens[i] as usize;
        mp.insert(data[kp..kp+kl].to_vec(), data[vp..vp+vl].to_vec());
        kp += kl;
        vp += vl;
    }
    Ok(mp)
}/* type Bi = (u32, u32); type TriBi = (Bi, Bi, Bi); type TriTriBi = (TriBi, TriBi, TriBi); // 64 * 3 * 3 *///enum MutualizationNode{Segment(Vec<(String, MutualizationNode)>), End(String)}

fn deshittify(txt: String) -> String {
    txt.replace("  ", " ").replace(" -", "").replace("- ", "")
    .replace(" \n ", "\n").replace(" ?", "?")
    .replace(" 'l", "'l").replace(" , ", ", ")
    .replace("I 'm", "I'm").trim().to_string()
}

fn demutualize(mutxt: String, state: Option<HashMap<String, String>>) -> Vec<String> {
    let mut itr = mutxt.into_chars();
    let mut buf = vec![];
    let mut appendances: Vec<String> = vec![];
    let mut stack: Vec<String> = vec![];
    let mut prependerances: Vec<String> = vec![];
    let mut out: Vec<String> = vec![];
    let mut variables = match state {
        None => HashMap::new(),
        Some(hm) => hm
    };
    let mut canset = true;
    loop {
        if let Some(ch) = itr.next() {
            match ch {
                '(' => { prependerances.push(buf.drain(..).collect()); }
                ')' => { prependerances.pop(); }
                '<' => {
                    if let Some(v) = variables.get(&buf.drain(..).collect::<String>()) {
                        out.push(deshittify(format!("{} {} {} {}", prependerances.join(" "), stack.join(" "), appendances.join(" "), v)));
                    }
                }
                '>' => {
                    if canset {
                        variables.insert(
                            buf.drain(..).collect(),
                            deshittify(format!("{} {} {}", prependerances.join(" "), stack.join(" "), appendances.join(" ")))
                        );
                    } else {
                        buf.clear();
                        canset = true;
                    }
                }
                '$' if buf.len() != 0 => {
                    if let Some(v) = variables.get(&buf.drain(..).collect::<String>()) {
                        buf.extend_from_slice(&v.chars().collect::<Vec<char>>());
                    }
                }
                '=' if buf.len() != 0 => {
                    if let Some(v) = variables.get(&buf.drain(..).collect::<String>()) {
                        canset = deshittify(format!("{} {} {}", prependerances.join(" "), stack.join(" "), appendances.join(" "))).eq(v);
                    }
                }
                '{' => { stack.push(buf.drain(..).collect()); }
                '~' => {
                    out.push(deshittify(format!("{} {} {} {}", prependerances.join(" "), stack.join(" "), appendances.join(" "), buf.drain(..).collect::<String>())));
                }
                '}' => {
                    if stack.len() == 0 {
                        out.push(deshittify(format!("{} {} {}", prependerances.join(" "), stack.join(" "), appendances.join(" "))));
                    }
                    stack.pop();
                }
                '[' => { appendances.push(buf.drain(..).collect()); }
                ']' => { appendances.pop(); }
                '.' => { out.push(deshittify(format!("{} {} {} {}", prependerances.join(" "), stack.join(" "), buf.drain(..).collect::<String>(), appendances.join(" ")))); }
                ' ' | '\n' if buf.len() == 0 || buf.last().is_some_and(|l| ch.eq(l) || '}'.eq(l)) => { continue; }
                _ => { buf.push(ch); }
            }
        } else { break; }
    }
    out
}

#[handler]
async fn mootroute(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    if let Ok(q) = req.parse_queries::<HashMap<String, String>>() {// let mut hm: HashMap<String, String> = HashMap::new();
        res.render(Text::Json(serde_json::to_string_pretty(&demutualize(req.parse_body().await?, Some(q)))?));
    } else {
        res.render(Text::Json(serde_json::to_string_pretty(&demutualize(req.parse_body().await?, None))?));
    }
    Ok(())
}

fn drain(mut it: MultimapValue<'_, &'static [u8]>) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut out = vec![];
    while let Some(r) = it.next() { out.push(r?.value().to_vec()); }
    Ok(out)
}
 
fn rm_list(ln: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
    let wx = MEMORY.begin_write()?;
    let vc = {
        let mut lists = wx.open_multimap_table(LISTS)?;
        drain(lists.remove_all(ln)?)?
    };
    wx.commit()?;
    Ok(vc)
}
 
/*fn get_notes_on(this: &[u8]) -> anyhow::Result<std::collections::HashMap<Vec<u8>, Vec<u8>>> {
    let rx = MEMORY.begin_read()?;
    let notes = rx.open_multimap_table(NOTES)?;
    let memories = rx.open_table(MEMORIES)?;
    let mut found = std::collections::HashMap::new();
    for k in drain(notes.get(this)?)? {
        if let Some(ag) = memories.get(k.as_slice())? {
            found.insert(k, ag.value().to_vec());
        }
    }
    if found.is_empty() {
        return Err(anyhow!("didn't find any notes with that"));
    }
    Ok(found)
}*/
 
fn rm_notes(on: &[u8]) -> anyhow::Result<()> {
    let wx = MEMORY.begin_write()?;
    {
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        for note in drain(notes.remove_all(on)?)? {
            noted_on.remove(note.as_slice(), on)?;
        }
    }
    wx.commit()?;
    Ok(())
}

/*
fn forget(this: &[u8]) -> anyhow::Result<Vec<u8>> {
    let wx = MEMORY.begin_write()?;
    let old = wx.open_table(MEMORIES)?.remove(this)?.map(|ag| ag.value().to_vec());
    let Some(quidity) = old else {
        wx.abort()?;
        return Err(anyhow!("there was nothing with that name anyways"));
    };
    {
        wx.open_multimap_table(TO_REUSE)?.insert(ALLOCATED, this)?;
        let mut notes = wx.open_multimap_table(NOTES)?;
        let mut noted_on = wx.open_multimap_table(NOTED_ON)?;
        for note in drain(notes.remove_all(this)?)? {
            noted_on.remove(note.as_slice(), this)?;
        }
        for on in drain(noted_on.remove_all(this)?)? {
            notes.remove(on.as_slice(), this)?;
        }
        let mut lists = wx.open_multimap_table(LISTS)?;
        let mut holding: Vec<Vec<u8>> = vec![];
        for r in lists.iter()? {
            let (ln, vals) = r?;
            for v in vals {
                if v?.value() == this {
                    holding.push(ln.value().to_vec());
                    break;
                }
            }
        }
        for ln in holding {
            lists.remove(ln.as_slice(), this)?;
        }
        let (f, r) = unsafe { gematria_dual_sum(quidity.as_slice()) };
        wx.open_multimap_table(MEMORY_GEMATRIA)?.remove(f as u32, this)?;
        wx.open_multimap_table(MEMORY_GEMATRIA_REVERSED)?.remove(r as u32, this)?;
        let mut memi = wx.open_multimap_table(MEMORY_INDEX)?;
        for tk in tokenate(quidity.as_slice()) {
            memi.remove(tk, this)?;
        }
        wx.open_table(EMBEDDINGS)?.remove(this)?;
    }
    wx.commit()?;
    Ok(quidity)
}
 */
// Sessions: SESSIONS (hash → (account, expiry)) is the record of truth; SESSIONS_LOOKUP (account → hashes)
// is kept in step with it. EXPIRIES_SESSIONS is no longer written or read: it was keyed by the expiry second
// alone, was never filled by make_session_for, and its retain_in(n..) kept the expired ones and dropped the live ones.
/*
fn make_session_for(id: u64, exp: u64) -> anyhow::Result<String> {
    let hashr = acc_hashr()?;
    let wtx = DB.begin_write()?;
    let sid = randstr(24);
    let h = hashr.hash(sid.as_bytes());
    let mut abort = false;
    {
        let mut sess = wtx.open_table(SESSIONS)?;
        if let Some(ag) = sess.insert(h.as_slice(), &(id, exp))? {
            abort = id != ag.value().0;
        }
        if !abort {
            wtx.open_multimap_table(SESSIONS_LOOKUP)?.insert(id, h.as_slice())?;
        }
    }
    if abort {
        wtx.abort()?;
        return Err(anyhow!("had to abort due to a rare collision, try again"));
    }
    wtx.commit()?;
    Ok(sid)
}
 
fn expiry_check() -> anyhow::Result<()> {
    let n = unix_now(0)?;
    let wx = DB.begin_write()?;
    {
        let mut sess = wx.open_table(SESSIONS)?;
        let mut sl = wx.open_multimap_table(SESSIONS_LOOKUP)?;
        let mut expired: Vec<(Vec<u8>, u64)> = vec![];
        for r in sess.iter()? {
            let (k, v) = r?;
            let (id, exp) = v.value();
            if exp <= n {
                expired.push((k.value().to_vec(), id));
            }
        }
        for (sid, id) in expired {
            sess.remove(sid.as_slice())?;
            sl.remove(id, sid.as_slice())?;
        }
    }
    wx.commit()?;
    Ok(())
}*/
 
#[handler]
async fn signout(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    let (id, sid_hashed) = auth_check_with_sid_hashed(req, res)?;
    res.remove_cookie("auth");
    let wtx = DB.begin_write()?;
    {
        wtx.open_table(SESSIONS)?.remove(sid_hashed.as_slice())?;
        wtx.open_multimap_table(SESSIONS_LOOKUP)?.remove(id, sid_hashed.as_slice())?;
    }
    wtx.commit()?;
    res.status_code(StatusCode::ACCEPTED);
    res.headers_mut().insert("Content-Type", "text/plain".parse().unwrap());
    res.body("signed out");
    Ok(())
}
 
fn rm_account(id: u64) -> anyhow::Result<()> {
    let wtx = DB.begin_write()?;
    {
        let mut accs = wtx.open_table(ACCOUNTS)?;
        if accs.remove(id)?.is_some() {
            let mut sess = wtx.open_table(SESSIONS)?;
            let mut sl = wtx.open_multimap_table(SESSIONS_LOOKUP)?;
            for sid in drain(sl.remove_all(id)?)? {
                sess.remove(sid.as_slice())?;
            }
        }
    }
    wtx.commit()?;
    Ok(())
}
 
fn remove_order(id: &str) -> anyhow::Result<(u64, Vec<(String, u32)>, String, String, u8, Vec<u64>, Vec<String>)> {
    let wx = DB.begin_write()?;
    let o_order = {
        let mut orders = wx.open_table(ORDERS)?;
        let o = orders.remove(id)?.map(|ag| ag.value());
        if let Some(o) = &o {
            wx.open_multimap_table(ACC_ORDER_LOOKUP)?.remove(o.0, id)?;
        }
        o
    };
    match o_order {
        Some(o) => {
            wx.commit()?;
            Ok(o)
        }
        None => {
            wx.abort()?;
            Err(anyhow!("unable to remove order, it did not exist in the db"))
        }
    }
}
 
/*fn change_order_status(id: &str, status: u8) -> anyhow::Result<()> {
    let wx = DB.begin_write()?;
    let found = {
        let mut orders = wx.open_table(ORDERS)?;
        let o = orders.get(id)?.map(|ag| ag.value());
        if let Some(mut order) = o {
            order.4 = status;
            orders.insert(id, &order)?;
            true
        } else {
            false
        }
    };
    if !found {
        wx.abort()?;
        return Err(anyhow!("no order with that id"));
    }
    wx.commit()?;
    Ok(())
}*/
 
fn textuality_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 96 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
 
#[handler]
async fn save_textuality(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let name = req.param::<String>("name").unwrap_or_default();
    if !textuality_name(&name) {
        FWD::bad_req(res, anyhow!("map name is 1 to 96 of A-Z a-z 0-9 _ -"));
        return Ok(());
    }
    let body = req.payload_with_max_size(64 * 1024 * 1024).await?.to_vec();
    if let Err(e) = serde_json::from_slice::<serde_json::Value>(&body) {
        FWD::bad_req(res, anyhow!("not json: {}", e));
        return Ok(());
    }
    tokio::fs::create_dir_all("./assets/spatiotextualities").await?;
    tokio::fs::write(format!("./assets/spatiotextualities/{}.json", name), &body).await?;
    res.status_code(StatusCode::ACCEPTED);
    res.render(Text::Plain(format!("written {}", name)));
    Ok(())
}
 
#[handler]
async fn list_textualities(req: &mut Request, res: &mut Response) -> anyhow::Result<()> {
    auth_check_admin(req, res)?;
    let mut names = vec![];
    let mut rd = match tokio::fs::read_dir("./assets/spatiotextualities").await {
        Ok(rd) => rd,
        Err(e) => {
            FWD::server(res, anyhow!("{}", e));
            return Ok(());
        }
    };
    while let Some(e) = rd.next_entry().await? {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                names.push(s.to_string());
            }
        }
    }
    names.sort();
    res.render(Text::Json(serde_json::to_string(&names)?));
    Ok(())
}


/*
//const xyzrgb: MultimapTableDefinition<(u16, u16, u16, u8, u8, u8), &[u8]> = MultimapTableDefinition::new("P");

const R: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("R");
const G: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("G");
const B: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("B");
const GB: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("GB");
const RB: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("RB");
const RG: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("RG");
const RGB: MultimapTableDefinition<(u8, u16, u16), &[u8]> = MultimapTableDefinition::new("RGB");

const MUTUALIZED: MultimapTableDefinition<&[u8], (u16, &[u8])> = MultimapTableDefinition::new("mutualized");

fn bytelr(input: Vec<u8>) -> Vec<u8> {
    let mut out = vec![];
    let mut stack: Vec<Vec<u8>> = vec![];
    let mut buf: Vec<u8> = vec![];
    let mut n: usize = 0;
    let mut variables: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    loop {
        if let Some(b) = input.get(n) {
            match b {
                255 => {

                }
                254 => {

                }
                253 => {

                }
                252 => {

                }
                251 => {

                }
                250 => {

                }
                249 => {

                }
                248 => {

                }
                247 => {

                }
                246 => {

                }
                245 => {

                }
                244 => {

                }
                243 => {

                }
                242 => {

                }
                241 => {

                }
                240 => {

                }
                239 => {

                }
                238 => {

                }
                237 => {

                }
                236 => {

                }
                235 => {

                }
                234 => {
                    
                }
                233 => {

                }
                232 => {

                }
                231 => {

                }
                230 => {

                }
                229 => {

                }
                228 => {

                }
                227 => {

                }
                226 => {

                }
                225 => {

                }
                224 => {
                    
                }
                223 => {

                }
                222 => {

                }
                221 => {

                }
                220 => {

                }
                219 => {

                }
                218 => {

                }
                217 => {

                }
                216 => {

                }
                215 => {

                }
                214 => {

                }
                213 => {

                }
                212 => {

                }
                211 => {

                }
                210 => {

                }
                209 => {

                }
                208 => {

                }
                207 => {

                }
                206 => {

                }
                205 => {

                }
                204 => {

                }
                203 => {

                }
                202 => {

                }
                201 => {

                }
                200 => {

                }
                199=> {

                }
                198=> {

                }
                197=> {

                }
                196=> {

                }
                195 => {

                }
                194 => {

                }
                193 => {

                }
                192 => {

                }
                191=> {

                }
                190=> {

                }
                189=> {

                }
                188=> {

                }
                187=> {

                }
                186=> {

                }
                185=> {

                }
                184=> {

                }
                183=> {

                }
                182=> {

                }
                181=> {

                }
                180=> {

                }
                179=> {

                }
                178=> {

                }
                177=> {

                }
                176=> {

                }
                175=> {

                }
                174=> {

                }
                173=> {

                }
                172=> {

                }
                171=> {

                }
                170=> {

                }


                169 => {

                }
                168 => {

                }
                167 => {
                    
                }
                166 => {
                    
                }
                165 => {
                    
                }
                164 => {

                }
                163 => {

                }
                162 => {

                }
                161 => {

                }
                160 => {

                }
                159 => {

                } 
                158 => {

                }
                157 => {

                }
                156 => {

                }
                155 => {

                }
                154 => {

                }
                153 => {

                }
                152 => {

                }
                151 => {

                }
                150 => {

                }
                149 => {

                }
                148 => {

                }
                147 => {

                }
                146 => {

                }
                145 => {

                }
                144 => {

                }
                143 => {

                }
                142 => {

                }         
                141 => {

                }
                140 => {

                }
                139 => {

                }
                138 => {

                }
                137 => {

                }
                136 => {

                }
                135 => {

                }
                134 => {
                    
                }
                133 => {

                }
                132 => {

                }
                131 => {

                }
                130 => {

                }
                129 => {

                }
                128 => {

                }
                127 => {

                }
                126 => {

                }
                125 => {

                }
                124 => {
                    
                }
                123 => {

                }
                122 => {

                }
                121 => {

                }
                120 => {

                }
                119 => {

                }
                118 => {

                }
                117 => {

                }
                116 => {

                }
                115 => {

                }
                114 => {

                }
                113 => {

                }
                112 => {

                }
                111 => {

                }
                110 => {

                }
                109 => {

                }
                108 => {

                }
                107 => {

                }
                106 => {

                }
                105 => {

                }
                104 => {

                }
                103 => {

                }
                102 => {

                }
                101 => {

                }
                100 => {

                }
                99=> {

                }
                98=> {

                }
                97=> {

                }
                96=> {

                }
                95 => {

                }
                94 => {

                }
                93 => {

                }
                92 => {

                }
                91=> {

                }
                90=> {

                }
                89=> {

                }
                88=> {

                }
                87=> {

                }
                86=> {

                }
                85=> {

                }
                84=> {

                }
                83=> {

                }
                82=> {

                }
                81=> {

                }
                80=> {

                }
                79=> {

                }
                78=> {

                }
                77=> {

                }
                76=> {

                }
                75=> {

                }
                74=> {

                }
                73=> {

                }
                72=> {

                }
                71=> {

                }
                70=> {

                }
                69=> {

                }
                68=> {

                }
                67=> {

                }
                66=> {

                }
                65=> {

                }
                64=> {

                }
                63=> {

                }
                62=> {

                }
                61=> {

                }
                60 => {

                }
                59 => {

                } 
                58 => {

                }
                57 => {

                }
                56 => {

                }
                55 => {

                }
                54 => {

                }
                53 => {

                }
                52 => {

                }
                51 => {

                }
                50 => {

                }
                49 => { // * flips page over, writes further * 
                    stack.pop();
                }
                48 => { // type-writer-ding
                    let fresh_what = buf.drain(..).collect::<Vec<u8>>();
                    match fresh_what.as_slice() {
                        &[19, 1, 21, 12] => {
                            println!("moo {:?}", stack);
                        }
                        _ => {

                        }
                    }
                    stack.push(fresh_what);
                }
                47 => {
                    buf.push(b'^');
                }
                46 => {
                    buf.push(b'=');
                }
                45 => {
                    buf.push(b'/');
                }
                44 => {
                    buf.push(b'-');
                }
                43 => {
                    buf.push(b'+');
                }
                42 => {
                    buf.push(b'*');
                }         
                41 => {
                    buf.push(b'9');
                }
                40 => {
                    buf.push(b'8');
                }
                39 => {
                    buf.push(b'7');
                }
                38 => {
                    buf.push(b'6');
                }
                37 => {
                    buf.push(b'5');
                }
                36 => {
                    buf.push(b'4');
                }
                35 => {
                    buf.push(b'3');
                }
                34 => {
                    buf.push(b'2');
                }
                33 => {
                    buf.push(b'1');
                }
                32 => {
                    buf.push(b'0');
                }
                31 => {
                    buf.push(b'#');
                }
                30 => {
                    buf.push(b'!');
                }
                29 => {
                    buf.push(b'\n');
                }
                28 => {
                    buf.push(b','); 
                }
                27 => {
                    buf.push(b'.');
                }
                26 => {
                    buf.push(b'z');
                }
                25 => {
                    buf.push(b'y');
                }
                24 => {
                    buf.push(b'x');
                }
                23 => {
                    buf.push(b'w');
                }
                22 => {
                    buf.push(b'v');
                }
                21 => {
                    buf.push(b'u');
                }
                20 => {
                    buf.push(b't');
                }
                19 => {
                    buf.push(b's');
                }
                18 => {
                    buf.push(b'r');
                }
                17 => {
                    buf.push(b'q');
                }
                16 => {
                    buf.push(b'p');
                }
                15 => {
                    buf.push(b'o');
                }
                14 => {
                    buf.push(b'n');
                }
                13 => {
                    buf.push(b'm');
                }
                12 => {
                    buf.push(b'l');
                }
                11 => {
                    buf.push(b'k');
                }
                10 => {
                    buf.push(b'j');
                }
                9 => {
                    buf.push(b'i');
                }
                8 => {
                    buf.push(b'h');
                }
                7 => {
                    buf.push(b'g');
                }
                6 => {
                    buf.push(b'f');
                }
                5 => {
                    buf.push(b'e');
                }
                4 => {
                    buf.push(b'd');
                }
                3 => {
                    buf.push(b'c');
                }
                2 => {
                    buf.push(b'b');
                }
                1 => {
                    buf.push(b'a');
                }
                0 => {
                    buf.push(b' ');
                }
            }
        }
    }
    out
}*/


/*fn deserialize_list_of_lists(bytes: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
    let bl = bytes.len();
    let mut cursor = 0;
    let mut result = Vec::new();
    if bl < 4 { return Ok(result); }
    let count = u32::from_be_bytes(bytes[0..4].try_into()?);
    cursor += 4;
    for _ in 0..count {
        if cursor + 4 > bl { break; }
        let len = u32::from_be_bytes(bytes[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;
        if cursor + len > bl { break; }
        result.push(bytes[cursor..cursor+len].to_vec());
        cursor += len;
    }
    Ok(result)
}*/
use std::env;

use sfu_rs::sfu::{sessions, sfu};

#[tokio::main]
async fn main() -> Result<(), ()> {
    // let args: Vec<String> = env::args().collect();
    // let server_addr = &args[1];

    let mut sfu = sfu::Sfu::new();
    let (mut session, mut is_new) = sfu.get_or_new(sessions::Id::from("foobar"));
    println!("is_new: {}", is_new);
    println!("got session: {}", session.id());
    (session, is_new) = sfu.get_or_new(sessions::Id::from("foobar"));
    println!("is_new: {}", is_new);
    println!("got session: {}", session.id());

    Ok(())
}

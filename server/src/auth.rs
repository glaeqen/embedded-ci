use std::collections::HashMap;

use log::*;
use once_cell::sync::OnceCell;
use rocket::http::Status;
use rocket::request::{self, FromRequest, Request};
use serde::{Deserialize, Serialize};

static TOKEN: OnceCell<HashMap<String, String>> = OnceCell::new();

#[derive(Debug, Serialize, Deserialize)]
pub struct Token();

#[derive(Debug, Serialize, Deserialize)]
pub enum ApiTokenError {
    Missing,
    Invalid,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Token {
    type Error = ApiTokenError;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        if TOKEN.get().unwrap().is_empty() {
            debug!("No token, accepting all connections");

            return request::Outcome::Success(Token());
        }

        let token = req.headers().get_one("Authorization");
        match token {
            Some(token) => {
                debug!("Token: {}", token);

                if token.starts_with("Bearer ") {
                    let token = &token[7..];

                    for (_, accepted_token) in TOKEN.get().unwrap() {
                        if accepted_token == token {
                            return request::Outcome::Success(Token());
                        }
                    }
                }

                request::Outcome::Failure((Status::Unauthorized, ApiTokenError::Invalid))
            }
            None => request::Outcome::Failure((Status::Unauthorized, ApiTokenError::Missing)),
        }
    }
}

pub fn set_token(arg: HashMap<String, String>) {
    TOKEN.set(arg).unwrap()
}

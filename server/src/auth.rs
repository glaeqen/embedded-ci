use embedded_ci_server::{AuthName, AuthToken};
use log::*;
use once_cell::sync::OnceCell;
use rocket::http::Status;
use rocket::request::{self, FromRequest, Request};
use rocket::Response;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

static TOKEN: OnceCell<HashMap<AuthName, AuthToken>> = OnceCell::new();

#[derive(Debug, Serialize, Deserialize)]
pub struct Token;

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

            return request::Outcome::Success(Token);
        }

        let token = req.headers().get_one("Authorization");
        match token {
            Some(token) => {
                debug!("Token: {}", token);

                if token.starts_with("Bearer ") {
                    let token_to_check = &token[7..];

                    for (_, valid_token) in TOKEN.get().unwrap() {
                        if valid_token.0 == token_to_check {
                            return request::Outcome::Success(Token);
                        }
                    }
                }

                request::Outcome::Failure((Status::Unauthorized, ApiTokenError::Invalid))
            }
            None => request::Outcome::Failure((Status::Unauthorized, ApiTokenError::Missing)),
        }
    }
}

/// Returns an empty, default `Response`. Always returns `Ok`.
impl<'a, 'r: 'a> rocket::response::Responder<'a, 'r> for Token {
    fn respond_to(self, _: &rocket::request::Request<'_>) -> rocket::response::Result<'static> {
        Ok(Response::new())
    }
}

pub fn set_token(arg: HashMap<AuthName, AuthToken>) {
    TOKEN.set(arg).unwrap()
}

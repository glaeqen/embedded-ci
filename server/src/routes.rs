use crate::app::RunQueue;
use embedded_ci_server::{JobStatus, RunJob, Targets};
use log::*;
use rocket::{
    fairing::{Fairing, Info, Kind},
    get,
    http::Header,
    post,
    serde::json::Json,
    Request, Response, State,
};
use rocket_okapi::swagger_ui::{make_swagger_ui, SwaggerUIConfig};
use rocket_okapi::{openapi, openapi_get_routes};
use std::sync::{Arc, Mutex};

#[openapi]
#[post("/run_job", format = "application/json", data = "<job>")]
fn run_job(
    _token: crate::auth::Token,
    job: Json<RunJob>,
    run_queue: &State<Arc<Mutex<RunQueue>>>,
) -> Json<Result<u32, String>> {
    let mut app = run_queue.lock().unwrap();

    let id = app.register_job(job.0);

    debug!("Job with id {:?}", id);

    Json(id)
}

#[openapi]
#[get("/status/<id>")]
fn get_status(
    _token: crate::auth::Token,
    id: u32,
    run_queue: &State<Arc<Mutex<RunQueue>>>,
) -> Json<Result<JobStatus, String>> {
    let app = run_queue.lock().unwrap();

    Json(
        app.get_status(id)
            .clone()
            .ok_or(format!("ID {} did not exist", id)),
    )
}

#[openapi]
#[get("/")]
fn index(_token: crate::auth::Token, run_queue: &State<Arc<Mutex<RunQueue>>>) -> Json<Targets> {
    let targets = run_queue.lock().unwrap().get_targets().clone();
    println!("Targets: {:?}", targets);

    Json(targets)
}

#[openapi]
#[get("/token")]
fn test_token(_token: crate::auth::Token) -> Json<String> {
    Json("hello with token".to_string())
}

pub struct CORS;

#[rocket::async_trait]
impl Fairing for CORS {
    fn info(&self) -> Info {
        Info {
            name: "Attaching CORS headers to responses",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _request: &'r Request<'_>, response: &mut Response<'r>) {
        response.set_header(Header::new("Access-Control-Allow-Origin", "*"));
        response.set_header(Header::new(
            "Access-Control-Allow-Methods",
            "POST, GET, PATCH, OPTIONS",
        ));
        response.set_header(Header::new("Access-Control-Allow-Headers", "*"));
        response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
    }
}

pub async fn serve_routes(state: Arc<Mutex<RunQueue>>) -> Result<(), rocket::Error> {
    rocket::build()
        .attach(CORS)
        // .mount("/", routes![index])
        .mount(
            "/",
            openapi_get_routes![index, get_status, run_job, test_token],
        )
        .mount(
            "/swagger",
            make_swagger_ui(&SwaggerUIConfig {
                url: "/openapi.json".to_string(),
                ..Default::default()
            }),
        )
        .manage(state)
        .launch()
        .await
}

use crate::{
    app::{JobStatus, RunQueue, RunJob},
    target::Targets,
};
use log::*;
use rocket::{get, post, serde::json::Json, State};
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

pub async fn serve_routes(state: Arc<Mutex<RunQueue>>) -> Result<(), rocket::Error> {
    rocket::build()
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

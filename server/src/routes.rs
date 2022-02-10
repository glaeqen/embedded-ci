use crate::app::RunQueue;
use embedded_ci_server::{JobStatus, RunJob, Targets};
use rocket::{
    fairing::{Fairing, Info, Kind},
    get,
    http::Header,
    post, routes,
    serde::json::Json,
    Request, Response, State,
};
use std::sync::{Arc, Mutex};

#[post("/run_job", format = "application/json", data = "<job>")]
fn run_job(
    _token: crate::auth::Token,
    job: Json<RunJob>,
    run_queue: &State<Arc<Mutex<RunQueue>>>,
) -> Json<Result<u32, String>> {
    let mut app = run_queue.lock().unwrap();

    let id = app.register_job(job.0);

    Json(id)
}

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

#[get("/")]
fn index(_token: crate::auth::Token, run_queue: &State<Arc<Mutex<RunQueue>>>) -> Json<Targets> {
    let targets = run_queue.lock().unwrap().get_targets().clone();

    Json(targets)
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
        .mount("/", routes![index, get_status, run_job])
        .manage(state)
        .launch()
        .await
}

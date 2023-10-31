use embedded_ci_common::{job, JobStatus, ServerStatus, Targets, Uuid};
use rocket::{
    fairing::{Fairing, Info, Kind},
    get,
    http::{Header, Status},
    post,
    response::status::{Accepted, Custom},
    routes,
    serde::json::Json,
    Ignite, Request, Response, Rocket, State,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

#[derive(rocket::Responder)]
pub enum PostJobError {
    #[response(status = 400)]
    InvalidJob(Json<job::ValidationErrors>),
    #[response(status = 425)]
    TooManyJobs(()),
    #[response(status = 500)]
    InternalQueueClosed(()),
}

#[post("/job", format = "application/json", data = "<job_desc>")]
fn post_job(
    _token: crate::auth::Token,
    job_desc: Json<job::JobDesc>,
    register_job_tx: &State<mpsc::Sender<job::Job>>,
    server_status: &State<Arc<Mutex<ServerStatus>>>,
    targets: &State<Targets>,
) -> Result<Accepted<Json<job::Job>>, PostJobError> {
    let job =
        job::Job::from_desc(job_desc.0, &targets).map_err(|e| PostJobError::InvalidJob(Json(e)))?;
    match register_job_tx.try_send(job.clone()) {
        Ok(_) => {
            server_status.lock().unwrap().job_enqueued(job.id);
            Ok(Accepted(Json(job)))
        }
        Err(mpsc::error::TrySendError::Full(_)) => Err(PostJobError::TooManyJobs(())),
        Err(mpsc::error::TrySendError::Closed(_)) => Err(PostJobError::InternalQueueClosed(())),
    }
}

#[get("/job/by-id/<id>")]
fn get_job_by_id(
    _token: crate::auth::Token,
    id: Uuid,
    server_status: &State<Arc<Mutex<ServerStatus>>>,
    finished_job_queue: &State<Arc<Mutex<VecDeque<job::JobResult>>>>,
) -> Result<Custom<Json<job::JobResult>>, Custom<Json<JobStatus>>> {
    let server_status = server_status.lock().unwrap();
    match server_status.job_status(id) {
        v @ JobStatus::NotFound => Err(Custom(Status::NotFound, Json(v))),
        v @ JobStatus::InQueue => Err(Custom(Status { code: 425 }, Json(v))), // TooEarly
        v @ JobStatus::Running => Err(Custom(Status { code: 425 }, Json(v))), // TooEarly
        JobStatus::Finished => match finished_job_queue
            .lock()
            .unwrap()
            .iter()
            .find(|&j| j.id == id)
            .cloned()
        {
            Some(job_result) => return Ok(Custom(Status::Found, Json(job_result.clone()))),
            None => unreachable!(
                "Job finished in ServerStatus but not found in the finished queue - bug?"
            ),
        },
    }
}

#[get("/job/last")]
fn last_job(
    _token: crate::auth::Token,
    finished_job_queue: &State<Arc<Mutex<VecDeque<job::JobResult>>>>,
) -> Result<Custom<Json<job::JobResult>>, Status> {
    finished_job_queue
        .lock()
        .unwrap()
        .back()
        .map(|v| Custom(Status::Found, Json(v.clone())))
        .ok_or_else(|| Status::NotFound)
}

#[get("/status")]
fn status(
    _token: crate::auth::Token,
    server_status: &State<Arc<Mutex<ServerStatus>>>,
) -> Json<ServerStatus> {
    let server_status = server_status.lock().unwrap();
    Json(server_status.clone())
}

#[get("/targets")]
fn targets(_token: crate::auth::Token, targets: &State<Targets>) -> Json<Targets> {
    Json(Clone::clone(&targets))
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

pub async fn serve(
    finished_job_queue: Arc<Mutex<VecDeque<job::JobResult>>>,
    register_job_tx: mpsc::Sender<job::Job>,
    targets: Targets,
    server_status: Arc<Mutex<ServerStatus>>,
) -> Result<Rocket<Ignite>, rocket::Error> {
    rocket::build()
        .attach(CORS)
        .mount(
            "/",
            routes![targets, post_job, get_job_by_id, status, last_job],
        )
        .manage(finished_job_queue)
        .manage(register_job_tx)
        .manage(targets)
        .manage(server_status)
        .launch()
        .await
}

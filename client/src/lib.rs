#![warn(missing_docs)]

//! Library providing the means of interfacing with the embedded CI server

pub mod builder;

use std::time::Duration;

use anyhow::anyhow;
pub use embedded_ci_common::*;
use reqwest::{StatusCode, Url};

/// Possible errors produced by the [`Client`]
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// A request error.
    #[error("A request failed")]
    Request(#[from] reqwest::Error),
    /// Unauthorized.
    #[error("Unauthorized: Token authentication failed")]
    Unauthorized,
    /// Invalid Job
    #[error("Job invalid: {0}")]
    InvalidJob(#[from] job::ValidationErrors),
    /// Generic error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

type Result<T> = core::result::Result<T, Error>;

/// REST API client for the embedded CI server
pub struct Client {
    server_url: Url,
    client: reqwest::Client,
    auth_token: Option<String>,
}

impl Client {
    /// Constructor
    ///
    /// Requires the URL and the authentication token (can be omitted if it's not
    /// required by the server)
    pub fn new(server_url: Url, auth_token: Option<String>) -> Self {
        Self {
            server_url,
            auth_token,
            client: reqwest::Client::new(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let rb = self
            .client
            .request(method, self.server_url.join(path).unwrap());
        match &self.auth_token {
            Some(auth_token) => rb.bearer_auth(auth_token),
            None => rb,
        }
    }

    async fn post_job(&self, desc: job::JobDesc) -> Result<job::Job> {
        let request_route = "/job";
        log::debug!("POST: {request_route}");
        let response = self
            .request(reqwest::Method::POST, request_route)
            .json(&desc)
            .send()
            .await?;
        let response_status = response.status();
        let result: core::result::Result<job::Job, job::ValidationErrors>;
        result = match response_status {
            StatusCode::ACCEPTED => Ok(response.json().await?),
            StatusCode::BAD_REQUEST => Err(response.json().await?),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized)?,
            status_code => Err(anyhow!("Unexpected status code: {status_code}"))?,
        };
        log::trace!("{request_route} response: {result:#?}");
        Ok(result?)
    }

    async fn poll_job_result(&self, job: job::Job) -> Result<job::JobResult> {
        let request_route = format!("/job/by-id/{}", job.id);
        let result = loop {
            log::debug!("GET: {request_route}");
            let response = self
                .request(reqwest::Method::GET, &request_route)
                .send()
                .await?;
            let response_status = response.status();
            match response_status {
                StatusCode::NOT_FOUND => Err(anyhow!(
                    "Job not found even though correctly enqueued, total server failure?"
                ))?,
                StatusCode::FOUND => break Ok(response.json().await?),
                status_code => {
                    if status_code.as_u16() == 425 {
                        // TooEarly
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    } else {
                        Err(anyhow!("Unexpected status code: {status_code}"))?
                    }
                }
            }
        };
        log::trace!("{request_route} response: {result:#?}");
        result
    }

    /// Run the job and wait for the result
    pub async fn run(&self, desc: job::JobDesc) -> Result<job::JobResult> {
        let job = self.post_job(desc).await?;
        self.poll_job_result(job).await
    }
}

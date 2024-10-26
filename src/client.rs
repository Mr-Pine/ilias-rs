use std::{borrow::Cow, fmt::Debug, path::{Path, PathBuf}};

use anyhow::{anyhow, Context, Ok, Result};
use file_storage_provider::FileStorageProvider;
use reqwest::{
    multipart::{self, Form, Part},
    Client, Response, Url,
};
use scraper::{Html, Selector};
use serde::{de::DeserializeOwned, Serialize};
use stream_download::{
    http::HttpStream, Settings, StreamDownload,
};
use tokio::runtime::Runtime;

mod file_storage_provider;

use super::Querypath;

#[derive(Debug)]
pub struct IliasClient {
    client: Client,
    runtime: Runtime,
    base_url: Url,
}

impl IliasClient {
    pub fn new(base_url: Url) -> Result<IliasClient> {
        let client = Client::builder().cookie_store(true).build()?;
        let runtime = Runtime::new().unwrap();

        Ok(IliasClient {
            client,
            runtime,
            base_url,
        })
    }

    pub fn get_querypath(&self, querypath: &str) -> Result<Html> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let text = self.runtime.block_on(async {
            let response = self.client.get(url).send().await?;
            let text = response.text().await?;
            Ok(text)
        })?;
        let html = Html::parse_document(&text);

        Ok(html)
    }

    pub fn post_querypath_form<T: Serialize + ?Sized + Debug>(
        &self,
        querypath: &str,
        form: &T,
    ) -> Result<Response> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let response = self
            .runtime
            .block_on(self.client.post(url).form(form).send())?;
        if response.url().as_str().contains("error") {
            return Err(anyhow!("Ilias error page"));
        }
        Ok(response.error_for_status()?)
    }

    pub fn get_text(&self, response: Response) -> Result<String> {
        Ok(self.runtime.block_on(response.text())?)
    }

    pub fn get_json<T: DeserializeOwned>(&self, response: Response) -> Result<T> {
        Ok(self.runtime.block_on(response.json())?)
    }

    pub fn post_querypath_multipart(
        &self,
        querypath: &str,
        form: multipart::Form,
    ) -> Result<Response> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let response = self
            .runtime
            .block_on(self.client.post(url).multipart(form).send())?;

        Ok(response.error_for_status()?)
    }

    pub fn download_file(&self, querypath: &str, to: PathBuf) -> Result<()> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        self.runtime.block_on(async {
            let stream = HttpStream::new(self.client.clone(), url).await?;
            let stream_download = StreamDownload::from_stream(
                stream,
                FileStorageProvider::new(to),
                Settings::default(),
            )
            .await?;

            Ok(stream_download)
        });

        println!("Downloaded (hopefully)");
        Ok(())
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<()> {
        println!("Authenticating!");

        let shib_path = "shib_login.php";

        let shib_params = [
            ("sendLogin", "1"),
            ("idp_selection", "https://idp.scc.kit.edu/idp/shibboleth"),
            ("il_target", ""),
            ("home_organization_selection", "Weiter"),
        ];

        let mut url = self.base_url.clone();
        url.set_path(shib_path);
        let shib_url = url.as_str().to_owned();

        let shib_login_page = self
            .runtime
            .block_on(self.client.post(url).form(&shib_params).send())?;

        let mut url = shib_login_page.url().to_owned();
        let is_ilias = url
            .as_str()
            .starts_with(self.base_url.host_str().context("Base url has no host")?);
        if is_ilias {
            println!("Exiting auth, already logged in");
            return Ok(());
        }

        let shib_login_fragment =
            Html::parse_document(self.runtime.block_on(shib_login_page.text())?.as_str());
        let csrf_selector =
            Selector::parse(r#"input[name="csrf_token"]"#).expect("Could not parse selector");
        let crsf_field = shib_login_fragment.select(&csrf_selector).next();

        let shib_continue_fragment: Html;

        let path_selector =
            Selector::parse(r#"form[method="post"]"#).expect("Could not parse selector");

        if crsf_field.is_some() {
            let crsf = crsf_field.unwrap().value().attr("value").unwrap();

            let form_data = [
                ("csrf_token", crsf),
                ("j_username", username),
                ("j_password", password),
                ("_eventId_proceed", ""),
            ];

            let post_querypath = shib_login_fragment
                .select(&path_selector)
                .next()
                .unwrap()
                .value()
                .attr("action")
                .unwrap();

            url.set_querypath(post_querypath);
            let continue_response = self
                .runtime
                .block_on(self.client.post(url).form(&form_data).send())?;

            shib_continue_fragment =
                Html::parse_document(self.runtime.block_on(continue_response.text())?.as_str());
        } else {
            shib_continue_fragment = shib_login_fragment;
        }

        let saml_selector =
            Selector::parse(r#"input[name="SAMLResponse"]"#).expect("Could not parse selector");
        let saml = shib_continue_fragment
            .select(&saml_selector)
            .next()
            .context("Did not find SAML Response input")?
            .value()
            .attr("value")
            .context("Could not get SAML response value")?;

        let continue_form_data = [("RelayState", shib_url.as_str()), ("SAMLResponse", saml)];

        let continue_url = shib_continue_fragment
            .select(&path_selector)
            .next()
            .unwrap()
            .value()
            .attr("action")
            .unwrap();

        let ilias_home = self
            .client
            .post(continue_url)
            .form(&continue_form_data)
            .send();
        let ilias_home = self.runtime.block_on(ilias_home);

        if ilias_home?.status().is_success() {
            println!("Logged in!");
            Ok(())
        } else {
            Err(anyhow!("Ilias login not successful!"))
        }
    }

    pub fn construct_file_part<T: AsRef<Path>>(&self, path: T) -> Result<Part> {
        Ok(self.runtime.block_on(Part::file(path))?)
    }
}

pub trait AddFileWithFilename {
    fn file_with_name<T, V>(self, name: T, file_part: Result<Part>, filename: V) -> Result<Form>
    where
        T: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>;
}

impl AddFileWithFilename for Form {
    fn file_with_name<T, V>(self, name: T, file_part: Result<Part>, filename: V) -> Result<Form>
    where
        T: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        Ok(self.part(name, file_part?.file_name(filename)))
    }
}

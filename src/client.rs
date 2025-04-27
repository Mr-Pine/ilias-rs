use std::{borrow::Cow, fmt::Debug, path::Path};

use log::info;
use reqwest::{
    multipart::{self, Form, Part},
    Client, Response, Url,
};
use scraper::{Html, Selector};
use serde::{de::DeserializeOwned, Serialize};
use snafu::{whatever, OptionExt, ResultExt, Whatever};
use tokio::{fs::File, io::BufWriter, runtime::Runtime};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

use super::Querypath;

#[derive(Debug)]
pub struct IliasClient {
    client: Client,
    runtime: Runtime,
    base_url: Url,
}

impl IliasClient {
    pub fn new(base_url: Url) -> Result<IliasClient, Whatever> {
        let client = Client::builder()
            .cookie_store(true)
            .use_rustls_tls()
            .build()
            .whatever_context("Could not build reqwest client")?;
        let runtime = Runtime::new().unwrap();

        Ok(IliasClient {
            client,
            runtime,
            base_url,
        })
    }

    pub fn get_querypath(&self, querypath: &str) -> Result<Html, Whatever> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let text = self
            .runtime
            .block_on(async {
                let response = self
                    .client
                    .get(url.clone())
                    .send()
                    .await
                    .whatever_context(format!("No response for {url}"))?;
                let text = response
                    .text()
                    .await
                    .whatever_context(format!("Could not get text of response for {url}"))?;
                Result::<_, Whatever>::Ok(text)
            })
            .whatever_context("Could not get text for querypath")?;
        let html = Html::parse_document(&text);

        Ok(html)
    }

    pub fn post_querypath_form<T: Serialize + ?Sized + Debug>(
        &self,
        querypath: &str,
        form: &T,
    ) -> Result<Response, Whatever> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let response = self
            .runtime
            .block_on(self.client.post(url).form(form).send())
            .whatever_context("Could not post to querypath")?;
        if response.url().as_str().contains("error") {
            whatever!("Ilias error page");
        }
        Ok(response
            .error_for_status()
            .whatever_context("Response had an error status code")?)
    }

    pub fn get_text(&self, response: Response) -> Result<String, Whatever> {
        Ok(self
            .runtime
            .block_on(response.text())
            .whatever_context("Could not get text of response")?)
    }

    pub fn get_json<T: DeserializeOwned>(&self, response: Response) -> Result<T, Whatever> {
        Ok(self
            .runtime
            .block_on(response.json())
            .whatever_context("Could not get json from response")?)
    }

    pub fn is_alert_response(&self, response: Response) -> Result<bool, Whatever> {
        let html = Html::parse_document(&self.get_text(response)?);
        let selector = Selector::parse(".alert-danger").whatever_context("Could not parse selector")?;
        Ok(html.select(&selector).next().is_some())
    }

    pub fn post_querypath_multipart(
        &self,
        querypath: &str,
        form: multipart::Form,
    ) -> Result<Response, Whatever> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        let response = self
            .runtime
            .block_on(self.client.post(url).multipart(form).send())
            .whatever_context("Could not send multipart form")?;

        Ok(response
            .error_for_status()
            .whatever_context("Response had an error status code")?)
    }

    pub fn download_file(&self, querypath: &str, to: &Path) -> Result<(), Whatever> {
        let mut url = self.base_url.clone();
        url.set_querypath(querypath);

        self.runtime
            .block_on(async {
                let response = self
                    .client
                    .get(url.clone())
                    .send()
                    .await
                    .whatever_context("Could not get response for download url")?;
                let body_stream = response.bytes_stream();
                let body_stream = body_stream.map(|result| {
                    result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                });
                let mut body_reader = StreamReader::new(body_stream);

                let mut options = File::options();
                options.write(true);
                options.create(true);
                let file = options
                    .open(to)
                    .await
                    .whatever_context("Unable to open file")?;
                let mut file_writer = BufWriter::new(file);

                tokio::io::copy(&mut body_reader, &mut file_writer)
                    .await
                    .whatever_context("Could not copy reader to writer")?;
                Result::<_, Whatever>::Ok(())
            })
            .whatever_context("Could not download file")?;
        Ok(())
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<(), Whatever> {
        info!("Authenticating!");

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
            .block_on(self.client.post(url).form(&shib_params).send())
            .whatever_context("Could not send multipart form")?;

        let mut url = shib_login_page.url().to_owned();
        let is_ilias = url.as_str().starts_with(
            self.base_url
                .host_str()
                .whatever_context("Base url has no host")?,
        );
        if is_ilias {
            println!("Exiting auth, already logged in");
            return Ok(());
        }

        let shib_login_fragment = Html::parse_document(
            self.runtime
                .block_on(shib_login_page.text())
                .whatever_context("Could not get text for login page")?
                .as_str(),
        );
        let csrf_selector =
            Selector::parse(r#"input[name="csrf_token"]"#).expect("Could not parse selector");
        let csrf_field = shib_login_fragment.select(&csrf_selector).next();

        let shib_continue_fragment: Html;

        let path_selector =
            Selector::parse(r#"form[method="post"]"#).expect("Could not parse selector");

        if let Some(csrf_field) = csrf_field {
            let csrf = csrf_field
                .value()
                .attr("value")
                .whatever_context("Could not get csrf token")?;

            let form_data = [
                ("csrf_token", csrf),
                ("j_username", username),
                ("j_password", password),
                ("_eventId_proceed", ""),
            ];

            let post_querypath = shib_login_fragment
                .select(&path_selector)
                .next()
                .whatever_context("Could not get login querypath element")?
                .value()
                .attr("action")
                .whatever_context("Could not get login querypath action")?;

            url.set_querypath(post_querypath);
            let continue_response = self
                .runtime
                .block_on(self.client.post(url).form(&form_data).send())
                .whatever_context("Could not post login form")?;

            shib_continue_fragment = Html::parse_document(
                self.runtime
                    .block_on(continue_response.text())
                    .whatever_context("Could not get continuation page during login")?
                    .as_str(),
            );
        } else {
            shib_continue_fragment = shib_login_fragment;
        }

        let saml_selector =
            Selector::parse(r#"input[name="SAMLResponse"]"#).expect("Could not parse selector");
        let saml = shib_continue_fragment
            .select(&saml_selector)
            .next()
            .whatever_context("Did not find SAML Response input")?
            .value()
            .attr("value")
            .whatever_context("Could not get SAML response value")?;

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
        let ilias_home = self
            .runtime
            .block_on(ilias_home)
            .whatever_context("Could not get response for ilias home page");

        if ilias_home?.status().is_success() {
            info!("Logged in!");
            Result::<_, Whatever>::Ok(())
        } else {
            whatever!("Ilias login not successful!")
        }
    }

    pub fn construct_file_part<T: AsRef<Path>>(&self, path: T) -> Result<Part, Whatever> {
        let part = async {
            let path = path.as_ref();
            let file_name = path
                .file_name()
                .map(|filename| filename.to_string_lossy().into_owned());
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            let mime = mime_guess::from_ext(ext).first_or_octet_stream();
            let file = File::open(path)
                .await
                .whatever_context("Could not open file")?;
            let length = file
                .metadata()
                .await
                .whatever_context("Could not get file length")?
                .len();
            let field = Part::stream_with_length(file, length)
                .mime_str(mime.as_ref())
                .whatever_context("Could not add mime string")?;

            Result::<_, Whatever>::Ok(if let Some(file_name) = file_name {
                field.file_name(file_name)
            } else {
                field
            })
        };

        Ok(self
            .runtime
            .block_on(part)
            .whatever_context("Could not construct file part")?)
    }
}

pub trait AddFileWithFilename {
    fn file_with_name<T, V>(
        self,
        name: T,
        file_part: Result<Part, Whatever>,
        filename: V,
    ) -> Result<Form, Whatever>
    where
        T: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>;
}

impl AddFileWithFilename for Form {
    fn file_with_name<T, V>(
        self,
        name: T,
        file_part: Result<Part, Whatever>,
        filename: V,
    ) -> Result<Form, Whatever>
    where
        T: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        Ok(self.part(
            name,
            file_part
                .whatever_context("invalid file part")?
                .file_name(filename),
        ))
    }
}

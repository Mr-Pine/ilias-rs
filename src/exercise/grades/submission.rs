use std::sync::OnceLock;

use log::debug;
use regex::Regex;
use reqwest::multipart::Form;
use scraper::{ElementRef, Selector, selectable::Selectable};
use serde::Deserialize;
use snafu::{OptionExt, ResultExt, Whatever, whatever};

use crate::{
    client::{AddFileWithFilename, IliasClient},
    local_file::NamedLocalFile,
};

/// A submission of a user or team for an assignment that feedback can be uploaded to.
#[derive(Debug)]
pub struct GradeSubmission {
    pub identifier: String,
    pub file_feedback_querypath: String,
}

static DROPDOWN_ACTION_SELECTOR: OnceLock<Selector> = OnceLock::new();
static TEAM_ID_SELECTOR: OnceLock<Selector> = OnceLock::new();
static SIGNIN_NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();

static UPLOAD_FEEDBACK_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();
static POST_UPLOAD_FEEDBACK_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();
static UPLOAD_POST_SCRIPT_SELECTOR: OnceLock<Selector> = OnceLock::new();
static UPLOAD_POST_REGEX: OnceLock<Regex> = OnceLock::new();

impl GradeSubmission {
    /// Construct a submission from it's table row element.
    pub fn parse(element: ElementRef) -> Result<GradeSubmission, Whatever> {
        let dropdown_action_selector = DROPDOWN_ACTION_SELECTOR.get_or_init(|| {
            Selector::parse(".dropdown-menu button").expect("Could not parse selector")
        });
        let team_id_selector = TEAM_ID_SELECTOR.get_or_init(|| {
            Selector::parse("td:nth-child(2) div.small").expect("Could not parse selector")
        });
        let signin_name_selector = SIGNIN_NAME_SELECTOR.get_or_init(|| {
            Selector::parse("td:nth-child(3).std").expect("Could not parse selector")
        });
        let name_selector = NAME_SELECTOR.get_or_init(|| {
            Selector::parse("td:nth-child(2).std").expect("Could not parse selector")
        });

        let identifier = if let Some(team_id_element) = element.select(team_id_selector).next() {
            let team_id = team_id_element.text().collect::<String>();
            let team_id = team_id
                .trim()
                .strip_prefix("(")
                .whatever_context(format!("Unexpected team id (no prefix '(') {}", team_id))?;
            let team_id = team_id
                .strip_suffix(")")
                .whatever_context(format!("Unexpected team id (no suffix ')') {}", team_id))?;

            format!("Team {team_id}")
        } else if let Some(signin_name_element) = element.select(signin_name_selector).next()
            && signin_name_element.text().collect::<String>().contains("@")
            && let Some(name_element) = element.select(name_selector).next()
        {
            let signin_name: String = signin_name_element.text().collect();
            let signin_name = signin_name.trim();
            let name: String = name_element.text().collect();
            let name = name.trim().replace(", ", "_");
            format!("{name}_{signin_name}")
        } else {
            whatever!("This submission style is not yet supported");
        };

        let feedback_querypath = element
            .select(dropdown_action_selector)
            .filter_map(|button| button.attr("data-action"))
            .find(|&querypath| {
                querypath.contains("cmdClass=ilResourceCollectionGUI")
                    || querypath.contains("cmd=listFiles")
            })
            .whatever_context(format!("Did not find file feedback querypath for {identifier}"))?
            .to_string();

        Ok(GradeSubmission {
            identifier,
            file_feedback_querypath: feedback_querypath,
        })
    }

    pub fn upload(&self, file: NamedLocalFile, ilias_client: &IliasClient) -> Result<(), Whatever> {
        debug!("Uploading {:?} to {:?}", file, self);
        let upload_feedback_form_selector = UPLOAD_FEEDBACK_FORM_SELECTOR.get_or_init(|| {
            Selector::parse(".ilToolbarContainer form").expect("Could not parse selector")
        });
        let post_upload_feedback_form_selector = POST_UPLOAD_FEEDBACK_FORM_SELECTOR
            .get_or_init(|| Selector::parse(".modal-body form").expect("Could not parse selector"));
        let upload_post_script_selector = UPLOAD_POST_SCRIPT_SELECTOR.get_or_init(|| {
            Selector::parse("body script:last-child").expect("Could not parse selector")
        });
        let upload_post_regex = UPLOAD_POST_REGEX.get_or_init(|| {
            Regex::new(r".*il\.UI\.Input\.File\.init\([^']*'[^']*',[^']*'(?<querypath>[^']+)'.*")
                .expect("Could not parse cursed regex lol")
        });

        debug!(
            "Querypath for upload form: {}",
            self.file_feedback_querypath
        );
        let upload_page = ilias_client.get_querypath(&self.file_feedback_querypath)?;

        let script_element = upload_page
            .select(upload_post_script_selector)
            .next()
            .whatever_context("Did not find script that contains upload post querypath")?;
        let script = script_element.text().collect::<String>();
        let upload_querypath_captures = &upload_post_regex.captures(&script);

        if let Some(upload_querypath_captures) = upload_querypath_captures {
            let upload_querypath = &upload_querypath_captures["querypath"];

            debug!("Got upload querypath {}", upload_querypath);

            let post_upload_querypath = upload_page
                .select(post_upload_feedback_form_selector)
                .next()
                .whatever_context("Did not find form to upload feedback")?
                .attr("action")
                .whatever_context("Form did not have action")?;

            debug!("Got post upload querypath {}", post_upload_querypath);

            let form = Form::new()
                .file_with_name(
                    "new_file",
                    ilias_client.construct_file_part(&file.path),
                    file.name.clone(),
                )?
                .text("cmd[uploadFile]", "Hochladen");

            #[derive(Deserialize)]
            #[allow(dead_code)]
            struct UploadResponse {
                status: usize,
                message: String,
                resource_id: String,
            }

            let response = ilias_client
                .post_querypath_multipart(upload_querypath, form)
                .whatever_context("Could not send submission form")?
                .error_for_status()
                .whatever_context("Ilias returned an error")?;
            let response = ilias_client
                .get_json::<UploadResponse>(response)
                .whatever_context("Could not deserialize upload response")?;
            if response.status != 1 {
                whatever!("Error response for feedback upload")
            }
        } else {
            let upload_querypath = upload_page
                .select(upload_feedback_form_selector)
                .next()
                .whatever_context("Did not find form to upload feedback")?
                .attr("action")
                .whatever_context("Form did not have action")?;

            let form = Form::new()
                .file_with_name(
                    "new_file",
                    ilias_client.construct_file_part(&file.path),
                    file.name.clone(),
                )?
                .text("cmd[uploadFile]", "Hochladen");

            ilias_client
                .post_querypath_multipart(upload_querypath, form)
                .whatever_context("Could not send submission form")?;
        }
        Ok(())
    }
}

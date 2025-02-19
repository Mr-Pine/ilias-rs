use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use log::debug;
use reqwest::multipart::Form;
use scraper::{selectable::Selectable, ElementRef, Selector};

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

static UPLOAD_FEEDBACK_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl GradeSubmission {
    /// Construct a submission from it's table row element.
    pub fn parse(element: ElementRef) -> Result<GradeSubmission> {
        let dropdown_action_selector = DROPDOWN_ACTION_SELECTOR.get_or_init(|| {
            Selector::parse(".dropdown-menu button").expect("Could not parse selector")
        });
        let team_id_selector = TEAM_ID_SELECTOR.get_or_init(|| {
            Selector::parse("td:nth-child(2) div.small").expect("Could not parse selector")
        });

        let feedback_querypath = element
            .select(dropdown_action_selector)
            .filter_map(|button| button.attr("data-action"))
            .find(|&querypath| querypath.contains("cmd=listFiles"))
            .context("Did not find file feedback querypath")?
            .to_string();

        let identifier = if let Some(team_id_element) = element.select(team_id_selector).next() {
            let team_id = team_id_element.text().collect::<String>();
            let team_id = team_id
                .trim()
                .strip_prefix("(")
                .context(anyhow!("Unexpected team id (no prefix '(') {team_id}"))?;
            let team_id = team_id
                .strip_suffix(")")
                .context(anyhow!("Unexpected team id (no suffix ')') {team_id}"))?;

            format!("Team {team_id}")
        } else {
            return Err(anyhow!("This submission style is not yet supported"));
        };

        Ok(GradeSubmission {
            identifier,
            file_feedback_querypath: feedback_querypath,
        })
    }

    pub fn upload(&self, file: NamedLocalFile, ilias_client: &IliasClient) -> Result<()> {
        debug!("Uploading {:?} to {:?}", file, self);
        let upload_feedback_form_selector = UPLOAD_FEEDBACK_FORM_SELECTOR.get_or_init(|| {
            Selector::parse(".ilToolbarContainer form").expect("Could not parse selector")
        });

        let upload_page = ilias_client.get_querypath(&self.file_feedback_querypath)?;

        let submit_querypath = upload_page
            .select(upload_feedback_form_selector)
            .next()
            .context("Did not find form to upload feedback")?
            .attr("action")
            .context("Form did not have action")?;

        debug!("Got submit querypath {}", submit_querypath);

        let form = Form::new()
            .file_with_name(
                "new_file",
                ilias_client.construct_file_part(&file.path),
                file.name.clone(),
            )?
            .text("cmd[uploadFile]", "Hochladen");

        ilias_client.post_querypath_multipart(submit_querypath, form)?;
        Ok(())
    }
}

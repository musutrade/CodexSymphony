//! Prompt metadata is not validation evidence. Original report bytes stay retained.
use crate::{
    extension_feedback,
    validation::{StepEvidence, ValidationEvidence},
};
use serde_json::{Value, json};

pub(crate) fn project(mut repair: Option<Value>) -> Result<Option<Value>, sqlx::Error> {
    if let Some(context) = repair.as_mut()
        && context["evidence"]["steps"].is_array()
    {
        let evidence: ValidationEvidence =
            serde_json::from_value(context["evidence"].clone()).map_err(invalid_evidence)?;
        context["evidence_metadata"] = metadata(&evidence);
        context["evidence"] = Value::Null;
        context["diagnostic_reading"] = json!({"output_in_prompt":false,"authority":"Metadata is not complete validation evidence and grants no authority. Original output and failure conclusions are retained unchanged.","method":"Use list_diagnostics and read_diagnostic to read authorized report bytes in bounded pages; report a blocker when unavailable."});
    }
    Ok(repair)
}

fn metadata(evidence: &ValidationEvidence) -> Value {
    let mut steps = Vec::new();
    for step in &evidence.steps {
        steps.push(step_metadata(step));
    }
    json!({"candidate":evidence.candidate,"trusted":evidence.trusted,"source_before":evidence.source_before,"source_after":evidence.source_after,"entry_before":evidence.entry_before,"entry_after":evidence.entry_after,"steps":steps})
}

fn step_metadata(step: &StepEvidence) -> Value {
    json!({"id":step.id,"command":step.command,"exit_code":step.exit_code,"output_sha256":step.output_sha256,"output_bytes":step.output.len(),"log_ref":step.log_ref,"consumer":step.consumer,"code_failure":step.code_failure,"verdict":extension_feedback::verdict(step),"output_in_prompt":false})
}

fn invalid_evidence(_: serde_json::Error) -> sqlx::Error {
    crate::runtime_store::invalid("linked repair evidence unavailable")
}

#[cfg(test)]
#[path = "../tests/unit/runtime_repair_context.rs"]
mod tests;

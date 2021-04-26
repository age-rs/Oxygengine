use oxygengine::user_interface::raui::{
    core::{implement_props_data, prelude::*},
    material::prelude::*,
};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct TipProps {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub label: String,
}
implement_props_data!(TipProps);

pub fn tip(context: WidgetContext) -> WidgetNode {
    let WidgetContext { key, props, .. } = context;

    let tip_props = props.read_cloned_or_default::<TipProps>();

    let action_props = TextPaperProps {
        text: tip_props.action,
        variant: "roboto5".to_owned(),
        use_main_color: true,
        alignment_override: Some(TextBoxAlignment::Right),
        ..Default::default()
    };

    let label_props = TextPaperProps {
        text: tip_props.label,
        variant: "5".to_owned(),
        use_main_color: true,
        alignment_override: Some(TextBoxAlignment::Left),
        ..Default::default()
    };

    widget! {
        (#{key} horizontal_box: {props.clone()} [
            (#{"action"} text_paper: {action_props})
            (#{"label"} text_paper: {label_props})
        ])
    }
}

pub fn confirm_tip(context: WidgetContext) -> WidgetNode {
    widget! {
        (#{context.key} tip: {TipProps { action: "ENTER: ".to_owned(), label: "Confirm".to_owned() }})
    }
}

pub fn save_tip(context: WidgetContext) -> WidgetNode {
    widget! {
        (#{context.key} tip: {TipProps { action: "F5: ".to_owned(), label: "Save".to_owned() }})
    }
}

pub fn quit_tip(context: WidgetContext) -> WidgetNode {
    widget! {
        (#{context.key} tip: {TipProps { action: "ESC: ".to_owned(), label: "Quit".to_owned() }})
    }
}

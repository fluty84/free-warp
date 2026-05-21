use crate::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Fill, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::prelude::ColorU;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

const CARD_WIDTH: f32 = 200.;
const CARD_SPACING: f32 = 20.;
const CARD_PADDING: f32 = 24.;
const TITLE_FONT_SIZE: f32 = 22.;
const LABEL_FONT_SIZE: f32 = 14.;
const DESC_FONT_SIZE: f32 = 12.;

#[derive(Debug, Clone)]
pub enum ModePickerAction {
    SelectWarpCloud,
    SelectLiteLLMGateway,
}

pub enum ModePickerEvent {
    WarpCloudSelected,
    LiteLLMGatewaySelected,
}

#[derive(Default)]
struct CardMouseStates {
    warp_cloud: MouseStateHandle,
    litellm: MouseStateHandle,
}

pub struct ModePickerView {
    mouse_states: CardMouseStates,
}

impl ModePickerView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            mouse_states: Default::default(),
        }
    }
}

impl Entity for ModePickerView {
    type Event = ModePickerEvent;
}

impl TypedActionView for ModePickerView {
    type Action = ModePickerAction;

    fn handle_action(
        &mut self,
        action: &Self::Action,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            ModePickerAction::SelectWarpCloud => {
                ctx.emit(ModePickerEvent::WarpCloudSelected);
            }
            ModePickerAction::SelectLiteLLMGateway => {
                ctx.emit(ModePickerEvent::LiteLLMGatewaySelected);
            }
        }
    }
}

fn build_card_content(
    icon: &'static str,
    label: &'static str,
    description: &'static str,
    font_family: FamilyId,
    primary_text: ColorU,
    secondary_text: ColorU,
) -> Box<dyn Element> {
    let card_icon = Text::new_inline(icon, font_family, 28.)
        .with_color(primary_text.into())
        .finish();

    let card_label = Text::new_inline(label, font_family, LABEL_FONT_SIZE)
        .with_color(primary_text.into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish();

    let card_desc = Text::new_inline(description, font_family, DESC_FONT_SIZE)
        .with_color(secondary_text.into())
        .finish();

    Flex::column()
        .with_spacing(8.)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(card_icon)
        .with_child(card_label)
        .with_child(card_desc)
        .finish()
}

impl View for ModePickerView {
    fn ui_name() -> &'static str {
        "ModePickerView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();

        let bg = theme.background().into_solid();
        let surface = theme.surface_2().into_solid();
        let primary_text = internal_colors::text_main(theme, bg);
        let secondary_text = internal_colors::text_sub(theme, bg);
        let border_color = theme.surface_3().into_solid();

        // Title
        let title = Text::new_inline("How would you like to use AI?", font_family, TITLE_FONT_SIZE)
            .with_color(primary_text.into())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish();

        let make_card = |icon: &'static str,
                         label: &'static str,
                         description: &'static str,
                         mouse_state: MouseStateHandle,
                         action: ModePickerAction| {
            let action_clone = action.clone();
            Hoverable::new(mouse_state, move |_| {
                ConstrainedBox::new(
                    Container::new(build_card_content(
                        icon,
                        label,
                        description,
                        font_family,
                        primary_text,
                        secondary_text,
                    ))
                    .with_uniform_padding(CARD_PADDING)
                    .with_background(Fill::Solid(surface))
                    .with_border(Border::all(1.).with_border_fill(Fill::Solid(border_color)))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                    .finish(),
                )
                .with_width(CARD_WIDTH)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action_clone.clone());
            })
            .finish()
        };

        let warp_card = make_card(
            "☁",
            "Warp Cloud",
            "Sign in. Uses\nWarp's servers.",
            self.mouse_states.warp_cloud.clone(),
            ModePickerAction::SelectWarpCloud,
        );

        let litellm_card = make_card(
            "⚡",
            "LiteLLM Gateway",
            "No login. Your own\nOpenAI-compatible URL.",
            self.mouse_states.litellm.clone(),
            ModePickerAction::SelectLiteLLMGateway,
        );

        let cards_row = Flex::row()
            .with_spacing(CARD_SPACING)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(warp_card)
            .with_child(litellm_card)
            .finish();

        let inner = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(32.)
            .with_child(title)
            .with_child(cards_row)
            .finish();

        Container::new(Align::new(inner).finish())
            .with_background(Fill::Solid(bg))
            .finish()
    }
}

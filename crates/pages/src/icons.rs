//! The icons Rafi uses, drawn from Lucide (via `dioxus-icons`). Shared by the
//! Dioxus app and the server-rendered participant page.

use dioxus::{core::Attribute, prelude::*};
use dioxus_icons::{IconProps as LucideProps, IconSize, lucide};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I {
    Ticket,
    Sun,
    Moon,
    Monitor,
    Gift,
    Users,
    Copy,
    Trash,
    Check,
    X,
    Plus,
    Sparkles,
    ShieldCheck,
    Maximize,
    Download,
    ArrowLeft,
    Trophy,
    Sliders,
    Eye,
    EyeOff,
    Pencil,
    Phone,
    Zap,
    Lock,
    Dice,
    Play,
    Flag,
    Alert,
    More,
    Undo,
    QrCode,
}

impl I {
    /// The Lucide icon behind this one.
    fn lucide(self) -> fn(LucideProps) -> Element {
        match self {
            I::Ticket => lucide::Ticket,
            I::Sun => lucide::Sun,
            I::Moon => lucide::Moon,
            I::Monitor => lucide::Monitor,
            I::Gift => lucide::Gift,
            I::Users => lucide::Users,
            I::Copy => lucide::Copy,
            I::Trash => lucide::Trash,
            I::Check => lucide::Check,
            I::X => lucide::X,
            I::Plus => lucide::Plus,
            I::Sparkles => lucide::Sparkles,
            I::ShieldCheck => lucide::ShieldCheck,
            I::Maximize => lucide::Maximize,
            I::Download => lucide::Download,
            I::ArrowLeft => lucide::ArrowLeft,
            I::Trophy => lucide::Trophy,
            I::Sliders => lucide::SlidersVertical,
            I::Eye => lucide::Eye,
            I::EyeOff => lucide::EyeOff,
            I::Pencil => lucide::Pencil,
            I::Phone => lucide::Smartphone,
            I::Zap => lucide::Zap,
            I::Lock => lucide::Lock,
            I::Dice => lucide::Dice5,
            I::Play => lucide::Play,
            I::Flag => lucide::Flag,
            I::Alert => lucide::TriangleAlert,
            I::More => lucide::Ellipsis,
            I::Undo => lucide::Undo2,
            I::QrCode => lucide::QrCode,
        }
    }

    /// The `<svg>` element, hidden from assistive technology (the text beside
    /// an icon carries its meaning). Lucide icons hold no state, so calling the
    /// component directly is enough.
    fn element(self, class: &str) -> Element {
        self.lucide()(LucideProps {
            size: IconSize::default(),
            attributes: vec![
                Attribute::new("class", class.to_string(), None, false),
                Attribute::new("aria-hidden", "true", None, false),
            ],
        })
    }

    /// Complete `<svg>` markup, for places that need a string (SSR, JS).
    pub fn svg(self, class: &str) -> String {
        dioxus::ssr::render_element(self.element(class))
    }
}

/// An inline icon. `class` sets the size and colour (e.g. `size-5 text-primary`).
#[component]
pub fn Icon(i: I, #[props(default = "size-5".to_string())] class: String) -> Element {
    rsx! {
        span { class: "inline-flex shrink-0", {i.element(&class)} }
    }
}

#![feature(prelude_import)]
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;
pub enum RelationKind {
    Imports,
    Calls,
    Constructs,
    Contains,
    Implements,
    Defines,
    References,
    Extends,
    Links,
    Touches,
    Produces,
    Consumes,
    Binds,
    Emits,
}

trait NativeLabel {
    fn label(self)
    -> &'static str;
}

macro_rules! labels {
    ($ty:ident { $($variant:ident => $label:literal),* $(,)? }) =>
    {
        impl NativeLabel for $ty
        {
            fn label(self) -> &'static str
            { match self { $(Self::$variant => $label),* } }
        }
    };
}
impl NativeLabel for RelationKind {
    fn label(self) -> &'static str {
        match self {
            Self::Imports =>

                "imports",
            Self::Calls => "calls",
            Self::Constructs => "constructs",
            Self::Contains => "contains",
            Self::Implements => "implements",
            Self::Defines => "defines",
            Self::References => "references",
            Self::Extends => "extends",
            Self::Links => "links",
            Self::Touches => "touches",
            Self::Produces => "produces",
            Self::Consumes => "consumes",
            Self::Binds => "binds",
            Self::Emits => "emits",
        }
    }
}

fn main() {
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Imports.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n", RelationKind::Calls.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Constructs.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Contains.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Implements.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Defines.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::References.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Extends.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n", RelationKind::Links.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Touches.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Produces.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n",
                RelationKind::Consumes.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n", RelationKind::Binds.label()));
    };
    {
        ::std::io::_print(format_args!("{0}\n", RelationKind::Emits.label()));
    };
}

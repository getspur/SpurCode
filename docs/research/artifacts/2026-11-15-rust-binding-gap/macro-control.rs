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
    fn label(self) -> &'static str;
}

macro_rules! labels {
    ($ty:ident { $($variant:ident => $label:literal),* $(,)? }) => {
        impl NativeLabel for $ty {
            fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),* }
            }
        }
    };
}

labels!(RelationKind {
    Imports => "imports", Calls => "calls", Constructs => "constructs",
    Contains => "contains", Implements => "implements", Defines => "defines",
    References => "references", Extends => "extends", Links => "links",
    Touches => "touches", Produces => "produces", Consumes => "consumes",
    Binds => "binds", Emits => "emits",
});

fn main() {
println!("{}", RelationKind::Imports.label());
println!("{}", RelationKind::Calls.label());
println!("{}", RelationKind::Constructs.label());
println!("{}", RelationKind::Contains.label());
println!("{}", RelationKind::Implements.label());
println!("{}", RelationKind::Defines.label());
println!("{}", RelationKind::References.label());
println!("{}", RelationKind::Extends.label());
println!("{}", RelationKind::Links.label());
println!("{}", RelationKind::Touches.label());
println!("{}", RelationKind::Produces.label());
println!("{}", RelationKind::Consumes.label());
println!("{}", RelationKind::Binds.label());
println!("{}", RelationKind::Emits.label());
}
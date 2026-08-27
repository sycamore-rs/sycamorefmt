use sycamore::prelude::*;

#[component]
fn App() -> View {
    view! {
        div {
            p { "Value: " (state) }
            button(on:click=increment) { "+" }
        }
    }
}

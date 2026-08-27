use sycamore::prelude::*;

#[component]
fn App() -> View {
    view! {
        CustomButton(id="button-one", kind="button", class="red-button", style="background-color:red;", disabled=true) { "Button 2" }
    }
}

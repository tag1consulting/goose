extern crate proc_macro;

use proc_macro::*;
use quote::quote;
use syn::Ident;

struct Route {
    name: Ident,
    ast: syn::ItemFn,
}

impl Route {
    pub fn new(
        input: TokenStream,
    ) -> syn::Result<Self> {
        let ast: syn::ItemFn = syn::parse(input)?;
        let name = ast.sig.ident.clone();

        Ok(Self {
            name,
            ast,
        })
    }
}

#[proc_macro_derive(TaskSet)]
pub fn taskset_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_taskset(&ast)

}

fn impl_taskset(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {
        impl TaskSet for #name {
        }
    };
    gen.into()
}

#[proc_macro_attribute]
pub fn task(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let gen = match Route::new(item) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    // @TODO: actually use this, for now just mask an error
    let _ = gen.ast;
    let x = format!(r#"
        const {}: bool = true;
    "#, gen.name);
    x.parse().expect("Generated invalid tokens")
}

use kalosm::{language::*, *};

#[tokio::main]
async fn main() {
    let description = prompt_input("What is your character like? ").unwrap();
    let character_name = prompt_input("What is your character's name? ").unwrap();
    let character_description =
        format!("{description} The assistant will never reveal that it is an AI or assistant.");

    let mut model = Llama::new_chat();
    let constraints = LiteralParser::new(format!("(Responding as {}) ", character_name))
        .then(StopOn::new(model.end_assistant_marker().to_string()));
    let mut chat = Chat::builder(&mut model)
        .with_system_prompt(character_description)
        .constrain_response(move |_, _| constraints.clone())
        .map_bot_response(move |response, _| {
            response
                .trim_start_matches(&format!("(Responding as {}) ", character_name))
                .trim()
        })
        .build();

    loop {
        let output_stream = chat
            .add_message(prompt_input("\n> ").unwrap())
            .await
            .unwrap();
        print!("Bot: ");
        output_stream.to_std_out().await.unwrap();
    }
}

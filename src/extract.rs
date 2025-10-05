use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, Special};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use schemars::{JsonSchema, schema_for};
use std::io::Write;
use std::num::NonZeroU32;
use std::process::exit;
use chrono::FixedOffset;

use gbnf::{self, GrammarItem, TerminalSymbol};


fn gen_gbnf(schema: schemars::Schema, eos_token: String) -> String {
    let js = &serde_json::to_string(schema.as_value()).unwrap();
    let mut gram = gbnf::Grammar::from_json_schema(&js)
        .map_err(|err| {
            println!("{err}");
            err
        })
        .unwrap();
    for mut r in gram.items.iter_mut() {
        match &mut r {
            GrammarItem::LineBreak | GrammarItem::Comment(_) => {}
            GrammarItem::Rule(rule) => {
                if rule.lhs.name == "root".to_string() {
                    if let Some(last_rule) = rule.rhs.items.last_mut() {
                        *last_rule = gbnf::ProductionItem::Terminal(
                            TerminalSymbol {
                                value: eos_token.clone(),
                            },
                            gbnf::RepetitionType::One,
                        );
                    }
                }
            }
        }
    }
    println!("{}", gram.to_string());
    gram.to_string()
}

pub(crate) fn test_extraction(content: &str) {
    let model_path = "./gemma-3-4b-it-UD-Q8_K_XL.gguf";
    let mut backend = LlamaBackend::init().unwrap();
    //backend.void_logs();
    let params = LlamaModelParams::default().with_n_gpu_layers(40);

    let prompt = content.to_string();
    println!("{prompt}");
    let model =
        LlamaModel::load_from_file(&backend, model_path, &params).expect("unable to load model");

    let ctx_params = LlamaContextParams::default().with_n_ctx(Some(NonZeroU32::new(8192).unwrap()));
    let mut ctx = model
        .new_context(&backend, ctx_params)
        .expect("unable to create the llama_context");
    let tokens_list = model
        .str_to_token(&prompt, AddBos::Always)
        .unwrap_or_else(|_| panic!("failed to tokenize {prompt}"));
    let n_len = 1024;

    // create a llama_batch with size 512
    // we use this object to submit token data for decoding
    let mut batch = LlamaBatch::new(512, 1);

    let last_index = tokens_list.len() as i32 - 1;
    for (i, token) in (0_i32..).zip(tokens_list.clone().into_iter()) {
        // llama_decode will output logits only for the last token of the prompt
        let is_last = i == last_index;
        batch.add(token, i, &[0], is_last).unwrap();
    }
    ctx.decode(&mut batch).expect("llama_decode() failed");

    let mut n_cur = batch.n_tokens();

    // The `Decoder`
    let mut decoder = encoding_rs::UTF_8.new_decoder();

    let eos_string = &model
        .token_to_str(model.token_eos(), Special::Tokenize)
        .unwrap()
        .to_string();
    let grammar = gen_gbnf(eos_string.to_string());
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::grammar(&model, &grammar, "root").unwrap(),
        LlamaSampler::greedy()
    ]);

    while n_cur <= n_len {
        // sample the next token
        {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);

            //sampler.accept(token);

            // is it an end of stream?
            if token == model.token_eos() {
                eprintln!();
                break;
            }

            let output_bytes = model.token_to_bytes(token, Special::Tokenize).unwrap();
            // use `Decoder.decode_to_string()` to avoid the intermediate buffer
            let mut output_string = String::with_capacity(128);
            let _decode_result = decoder.decode_to_string(&output_bytes, &mut output_string, false);
            print!("{output_string}");
            std::io::stdout().flush().unwrap();

            batch.clear();
            batch.add(token, n_cur, &[0], true).unwrap();
        }

        n_cur += 1;

        ctx.decode(&mut batch).expect("failed to eval");
    }
}

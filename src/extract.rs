use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, Special};
use llama_cpp_2::sampling::LlamaSampler;
use schemars::Schema;
use serde_json::Value;
use std::io::Write;
use std::num::NonZeroU32;
use std::path::Path;

use gbnf::{self, GrammarItem, NonTerminalSymbol, ProductionItem, RepetitionType, TerminalSymbol};

use crate::types::FieldExtract;

fn gen_gbnf(schema: &schemars::Schema, eos_token: String) -> String {
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
    if let Some(p) = gram.recurring_items.get_mut(&NonTerminalSymbol {
        name: "ws".to_string(),
    }) {
        if let Some(last_item) = p.items.last_mut() {
            if let ProductionItem::CharacterSet(_, rep_type) = last_item {
                *rep_type = RepetitionType::One
            }
        }
    }
    gram.to_string()
}

pub(crate) struct CustomFieldModelExtractor {
    backend: LlamaBackend,
    model: LlamaModel,
    ctx_params: LlamaContextParams,
    sampler: LlamaSampler,
    eos_string: String,
}

impl CustomFieldModelExtractor {
    pub fn new(model_path: &Path, num_gpu_layers: usize, response_schema: &Schema) -> Self {
        let mut backend = LlamaBackend::init().unwrap();
        backend.void_logs();
        let params = LlamaModelParams::default().with_n_gpu_layers(num_gpu_layers as u32);
        let model = LlamaModel::load_from_file(&backend, model_path, &params)
            .expect("unable to load model");
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(NonZeroU32::new(8192).unwrap()))
            .with_n_batch(8192);

        let eos_string = &model
            .token_to_str(model.token_eos(), Special::Tokenize)
            .unwrap()
            .to_string();
        let grammar = gen_gbnf(response_schema, eos_string.to_string());
        let sampler = LlamaSampler::chain_simple([
            LlamaSampler::grammar(&model, &grammar, "root").unwrap(),
            LlamaSampler::dry(&model, 5., 1.75, 2, 1024, ["\n", ":", "\"", "*"]),
            LlamaSampler::greedy(),
        ]);

        Self {
            backend,
            model,
            ctx_params,
            sampler,
            eos_string: eos_string.to_string(),
        }
    }

    pub fn extract(&mut self, base_data: &Value, dry_run: bool) -> FieldExtract {
        self.sampler.reset();
        let prompt = format!("{}\n", serde_json::to_string(base_data).unwrap());
        let mut ctx = self
            .model
            .new_context(&self.backend, self.ctx_params.clone())
            .expect("unable to create the llama_context");
        let tokens_list = self
            .model
            .str_to_token(&prompt, AddBos::Always)
            .unwrap_or_else(|_| panic!("failed to tokenize {prompt}"));
        let n_len = tokens_list.len() + 4096;

        // create a llama_batch with size 512
        // we use this object to submit token data for decoding
        let mut batch = LlamaBatch::new(n_len, 1);

        let last_index = tokens_list.len() as i32 - 1;
        for (i, token) in (0_i32..).zip(tokens_list.clone().into_iter()) {
            // llama_decode will output logits only for the last token of the prompt
            let is_last = i == last_index;
            batch.add(token, i, &[0], is_last).unwrap();
        }
        ctx.decode(&mut batch).expect("llama_decode() failed");
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut n_cur = batch.n_tokens();
        let mut output = String::new();
        while n_cur as usize <= n_len {
            // sample the next token
            {
                let token = self.sampler.sample(&ctx, batch.n_tokens() - 1);

                //sampler.accept(token);

                // is it an end of stream?
                if token == self.model.token_eos() || output.ends_with(&self.eos_string) {
                    eprintln!();
                    break;
                }

                let output_bytes = self.model.token_to_bytes(token, Special::Tokenize).unwrap();
                // use `Decoder.decode_to_string()` to avoid the intermediate buffer
                let mut output_string = String::with_capacity(128);
                let _decode_result =
                    decoder.decode_to_string(&output_bytes, &mut output_string, false);
                if dry_run {
                    //println!("{output_string}\t\t token_cnt: {n_cur}");
                    print!("{output_string}");
                    if n_cur % 100 == 0 {
                        let _ = std::io::stdout().flush();
                    }
                }
                output.push_str(&output_string);

                batch.clear();
                batch.add(token, n_cur, &[0], true).unwrap();
            }

            n_cur += 1;

            ctx.decode(&mut batch).expect("failed to eval");
        }
        // remove eos token
        let output = output.replace(&self.eos_string, "");
        //println!("{output}");
        serde_json::from_str(&output).unwrap()
    }
}

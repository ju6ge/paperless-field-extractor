paperless-field-extractor
=========================

This project is an extension to the excellent [paperless-ngx](https://github.com/paperless-ngx/paperless-ngx) software.

# Who is this project for?

This project is for people who want to expand the machine learning capabilities of paperless without sacrificing privacy. The 
goal is to integrate language model capabilities into paperless seamlessly without yet another chat and prompting interface or 
complex user interface. This is a standalone project and does not require an extra service to provide model inference everything is baked in already.

If you are looking for document chat or don't care and are fine sending all your private documents to the big cloud providers checkout these projects:
- [paperless-gpt](https://github.com/icereed/paperless-gpt)
- [paperless-ai](https://github.com/clusterzx/paperless-ai)

## Under the Hood

Under the hood this software is running `llama.cpp` as an inference engine to provide a local language model without depending on any cloud providers. Depending on the selected feature it is possible to run
with `cuda`, `vulkan`, `openmp` or `native` acceleration.
As a base model this software is using a quantized version of `Qwen3` to reduce the resource requirements and enable running this even with limited resources.

Long term I want expand the features to enable fine tuning models to your document corpus. This is where the actual learning would come in.

# Custom Field Value Prediction

In its current form `paperless-ngx` does not support predicting the values of custom fields from the contents of the document. Addressing this is a complicated issue:

- As opposed to the currently predictable values for each document (`correspondent`, `document_date`, `storage_path`, `document_type`) custom fields do not exist for every document! 
Also the values of a custom field are much more flexible in what the content could be. Paperless uses classical neural nets to train predictors for the supported fields from the user specified 
variants. While document_date uses a complex regex + neural net approach. This is not easily generalizable to more field types such as the user defined custom fields.

- The first step in automating custom fields is assigning them to documents. However, since this requires the Paperless system to process the documents first, implementing this as a built-in feature would significantly complicate the standard workflow.

# How does this tool expect the workflow to work?

1. Documents are imported and processed by paperless default document import, assigning document types as usual
2. Use Paperless workflows to assign unfilled custom fields to the documents based on document_type and correspondent
3. Use this software to fill in the empty custom fields:
   - First all documents are scanned for unfilled custom fields and a given a processing tag, to indicate to the user that the document is being worked on
   - Uses a locally running language model to predict the value of the custom field from the document data
   - Upload filled custom fields to the corresponding document and set a finished tag to inform the user that all document processing has finished
   
# Supported Custom Field Types

Currently this projects predicting the following kinds of custom fields:
- [x] Boolean
- [x] Date
- [x] Integer
- [x] Number
- [x] Monetary
- [x] Text
- [x] Select
- [ ] Document Link
- [ ] URL
- [ ] LargeText


# Configuration

Configuration of the software is possible via a configuration file at `/etc/paperless-field-extractor/config.toml` or via environment variables. Environment variables can be used to overwrite values from the configuration file.

Apart from configuration an API Token is required to enable communication with the paperless API! This token should be made availible via the `PAPERLESS_API_CLIENT_API_TOKEN` environment variable!!!

This file shows the default configuration and explains the options:
``` toml
# corresponding env var `PAPERLESS_SERVER`, defines were the paperless instnace is reachable
paperless_server = "https://example-paperless.domain"
# corresponding env var `GGUF_MODEL_PATH`, defines where the gguf model file is located
model = "/usr/share/paperless-field-extractor/model.gguf"
# corresponding env var `NUM_GPU_LAYERS`, sets llama cpp option num_cpu_layers when initializing the inference backend zero here means unlimited
num_gpu_layers = 0

# correspondent suggesting enables the language model to process all inbox documents and add extra suggestions to the correspondet value, this is useful if you have a lot of new document that paperless has not trained for matching yet
# the corresponding environment var is `CORRESPONDENT_SUGGEST`
correspondent_suggestions = false

# corresponding env var `PROCESSING_TAG_NAME`, display name of the tag that is show when a document is being processed
processing_tag = "🧠 processing"
# corresponding env var `PROCESSING_TAG_COLOR`, display color of the tag that is show when a document is being processed
processing_color = "#ffe000"
# corresponding env var `FINISHED_TAG_NAME`, display name of the tag that is show when a document has been fully processed
finished_tag = "🏷️ finished"
# corresponding env var `FINISHED_TAG_COLOR`, display color of the tag that is show when a document has been fully processed
finished_color = "#40aebf"
# corresponding env var `PAPERLESS_USER`, default user to use when creating processing and finshed tags on inital connection
tag_user_name = "user"
```

# Setup

If you just want to run this software for your own instance using a containerized approach is recommended. 

## Containerized Approach

The default container is setup to include a model already and with some environment variables should be fully functional:

``` sh
<podman/docker> run -it --rm \
    --device /dev/kfd \ # give graphics device access to the container
    --device /dev/dri \ # give graphics device access to the container
    -e PAPERLESS_API_CLIENT_API_TOKEN=<token> \
    -e PAPERLESS_SERVER=<paperless_ngx_url> \
    -e PAPERLESS_USER=<user> \ # used for tag creation
    ghcr.io/ju6ge/paperless-field-extractor:<version>-<backend>
```

Currently only the `vulkan` backend has a prebuilt container availible, it should be fine for most deployments even without a graphics processor availible.

The easiest way to have this run in the background is to configure a cron job or systemd-timer to regularly run the software regularly checking for new documents with unfilled custom fields.

## Dry Run for Testing

If you wish to check how this would look for your documents with unfilled custom fields you can use the dry-run mode.

``` sh
<podman/docker> run -it --rm \
    --device /dev/kfd \ # give graphics device access to the container
    --device /dev/dri \ # give graphics device access to the container
    -e PAPERLESS_API_CLIENT_API_TOKEN=<token> \
    -e PAPERLESS_SERVER=<paperless_ngx_url> \ 
    -e PAPERLESS_USER=<user> \ # used for tag creation
    ghcr.io/ju6ge/paperless-field-extractor:<version>-<backend> --dry-run
```

This will run the inference printing the results to the terminal, but without setting add tags to documents or sending the extracted fields back to paperless. This mode is also useful for evaluing differnt
models.

NOTE: The processing and finshed tags will be setup as tags on the server though, since the software assumes requires their existence.

## Building the Container yourself

You can also build the container locally if you prefer. For this the following command will do the trick:

``` sh
<podman/docker> build \
    -f distribution/docker/Dockerfile \
    -t localhost/paperless-field-extractor:vulkan \
    --build-arg INFERENCE_BACKEND=<backend> \  #this argument is required to select the compute backend, cuda is currenly not supported by the docker build 
    --build-arg MODEL_URL=<url> \  #optionaly you can point the build process to include a different gguf model by providing a download url
    --build-arg MODEL_LICENSE_URL=<url> \  #if you change the model, consider including its license in the container build 
    .
```

# Build from Source

For development or advanced users manual compilation and setup may be desired.

Successfull building requires selecting a compute backend via feature flag:

``` sh
cargo build --release -F <backend>
```

You can select from the following backends:
- native (CPU)
- openmp (CPU)
- cuda (GPU)
- vulkan (CPU + GPU)

Depending on your selection you will need to have the corresponding system libraries installed on your device, with development headers included.

Afterward building you can setup a config file at `/etc/paperless-field-extractor/config.toml` and run the software. 
You will need to download a model gguf yourself and configure the `GGUF_MODEL_PATH` environment variable or `model` config option to point to its location!

# Future Work

Depending on interesent and request the following future updates may come:
- Contious Serving, using Webhooks to automatically trigger custom field extraction instead of requiring a timer setup
- Automated Finetuning using LoRa on existing corpus of documents

# LICENSE

This software is licensed under the AGPL-3.0

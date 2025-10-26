paperless-field-extractor
=========================

This project is an extension to the excellent [paperless-ngx](https://github.com/paperless-ngx/paperless-ngx) software.

In its current form `paperless-ngx` does not support predicting the values of custom fields from the contents of the document. Addressing this is a complicated issue:

- As opposed to the currently predictable values for each document (`correspondent`, `document_date`, `storage_path`, `document_type`) custom fields do not exist for every document! 
Also the values of a custom field are much more flexible in what the content could be. Paperless uses classical neural nets to train predictors for the supported fields from the user specified 
variants. While document_date uses a complex regex + neural net approach. This is not easily generalizable to more field types such as the user defined custom fields.

- The first step in automating custom fields is assigning them to documents. However, since this requires the Paperless system to process the documents first, implementing this as a built-in feature would significantly complicate the standard workflow.

# How does this tool expect the workflow to work?

1. Documents are imported and processed by paperless default document import, assigning document types as usual
2. Use Paperless workflows to assign unfilled custom fields to the documents based on document_type and correspondent
3. Use this software to fill in the empty custom fields:
   - First all documents are scannend for unfilled custom fields and a given a processing tag, to indicate to the user that the document is being worked on
   - Uses a locally running language model to predict the value of the custom field from the document data
   - Upload filled custom fields to the corresponding document and set a finished tag to inform the user that all document processing has finished

# Under the Hood

Under the hood this software is running `llama.cpp` as an inference engine to provide a local language model without depending on any cloud providers. Depending on the selected feature it is possible to run
with `cuda`, `vulkan`, `openmp` or `native` excelleration.
As a base model this software is using a quantized version of Qwen3 to reduce the resource requirements and enable running this even with limited resources.

# Setup

# Configuration

# LICENSE

This software is licensed under the AGPL-3.0

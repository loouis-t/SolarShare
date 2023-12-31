// Code generated by the multiversx-sc multi-contract system. DO NOT EDIT.

////////////////////////////////////////////////////
////////////////// AUTO-GENERATED //////////////////
////////////////////////////////////////////////////

// Init:                                 1
// Endpoints:                           10
// Async Callback (empty):               1
// Total number of exported functions:  12

#![no_std]
#![feature(alloc_error_handler, lang_items)]

multiversx_sc_wasm_adapter::allocator!();
multiversx_sc_wasm_adapter::panic_handler!();

multiversx_sc_wasm_adapter::endpoints! {
    solar_share
    (
        set_up_contract
        contract_electricity_provider
        lock_esdt_amount
        contract_status
        claim_refund
        provider_account_status
        claim_providers_due
        getProviderDetails
        getContractDetails
        getContractTokenIdentifier
    )
}

multiversx_sc_wasm_adapter::empty_callback! {}

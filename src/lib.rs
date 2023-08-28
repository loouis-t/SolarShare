#![no_std]

multiversx_sc::imports!();
multiversx_sc::derive_imports!();

#[derive(TopEncode, TopDecode, TypeAbi, PartialEq, Eq, Clone, Copy, Debug)]
pub enum ContractStatus {
    Terminable,
    ClientEsdtAmountClaimable,
    Terminated,
}

#[derive(TopEncode, TopDecode, TypeAbi, PartialEq, Eq, Clone, Copy, Debug)]
pub enum ProviderAccountStatus {
    Empty,
    EsdtAmountClaimable,
}

#[derive(TopEncode, TopDecode, TypeAbi, PartialEq, Eq, Clone, Debug)]
pub struct ContractDetails<M: ManagedTypeApi> {
    pub deadline: u64,
    pub client_esdt_amount_locked: BigUint<M>,
}

#[derive(NestedEncode, NestedDecode, TopEncode, TopDecode, TypeAbi)]
pub struct ProviderDetails<M: ManagedTypeApi> {
    pub sharing_module_address: ManagedAddress<M>,
    pub deadline: u64,
    pub kwh_price: BigUint<M>,
    pub due_amount: BigUint<M>,
}

#[multiversx_sc::contract]
pub trait SolarShare {
    // init SC: Propriétaire SC choisit le token utilisé
    #[init]
    fn init(&self, token_identifier: EgldOrEsdtTokenIdentifier) {
        // Stocker token utilisé si c'est un token qui existe
        require!(token_identifier.is_valid(), "Invalid token provided");
        self.cf_token_identifier().set(&token_identifier);
    }

    // Permettre à un producteur de proposer un contrat à des clients
    // producteur choisit : prix kWh
    #[endpoint]
    fn set_up_contract(&self, kwh_price: BigUint, deadline: u64) {
        let caller = self.blockchain().get_caller();

        require!(kwh_price > 0, "kWh price must be > 0");
        require!(
            deadline > self.get_current_time(),
            "end of contract must be later than today"
        );

        // mettre à jour :
        // - deadline souhaitée par producteur
        // - prix par kwh
        // - montant d (inchangé)
        let due_amount;
        if self.provider(&caller).is_empty() {
            due_amount = BigUint::zero();
        } else {
            due_amount = self.provider(&caller).get().due_amount;
        }

        let sharing_module_address = self.provider(&caller).get().sharing_module_address;

        let details = ProviderDetails {
            sharing_module_address,
            deadline,
            kwh_price,
            due_amount,
        };
        self.provider(&caller).set(&details);
    }

    // client souscrit un contrat aupres d'un fournisseur
    #[endpoint]
    fn contract_electricity_provider(&self, provider: &ManagedAddress, deadline: u64) {
        let client = self.blockchain().get_caller();
        let current_contractors = (provider.clone(), client.clone());
        require!(
            !self.provider(provider).is_empty(),
            "provided address is not a registered electricity provider or not presenting contract"
        );
        require!(
            deadline > self.get_current_time(),
            "wished end of contract must be later than today"
        );
        require!(
            self.provider(provider).get().deadline > self.get_current_time(),
            "cannot contract, contract is not up to date (deadline in the past)"
        );
        require!(
            deadline <= self.provider(provider).get().deadline,
            "contract deadline must be lower or equal to provider's deadline"
        );
        require!(
            self.contract(&current_contractors).is_empty(),
            "existing contract found for provided (provider, client)"
        );
        // Valider contrat entre provider et client et enregistrer sa deadline
        let details = ContractDetails {
            deadline,
            client_esdt_amount_locked: BigUint::zero(),
        };
        self.contract(&current_contractors).set(&details);
    }

    // client bloque fonds sur SC pour garantir paiement provider
    #[endpoint]
    #[payable("*")]
    fn lock_esdt_amount(&self, provider: &ManagedAddress) {
        let caller = self.blockchain().get_caller();
        let current_contractors = (provider.clone(), caller.clone());

        require!(
            !self.provider(provider).is_empty(),
            "can't lock esdts for non provider address"
        );
        require!(
            self.contract_status(provider, &caller) == ContractStatus::Terminable,
            "cannot lock funds for terminated contract"
        );

        // Récup infos sur paiement de ce SC
        let (token, _, esdt_amount_transferred) =
            self.call_value().egld_or_single_esdt().into_tuple();
        require!(token == self.cf_token_identifier().get(), "wrong token");

        // Check si paiement correspond bien à [durée contrat (en heures)] * [prix kWh]
        let contract = self.contract(&current_contractors).get();
        let contract_remaining_hours =
            BigUint::from((contract.deadline - self.get_current_time()) / 3600);

        require!(
            esdt_amount_transferred
                >= self.provider(provider).get().kwh_price * contract_remaining_hours,
            "amount locked must be at least the required amount for contract period"
        );

        // save contract details
        self.contract(&current_contractors)
            .update(|contract| contract.client_esdt_amount_locked += &esdt_amount_transferred);
    }

    // uniquement appelable par le module de partage d'éléctricité
    #[endpoint]
    fn share_electricity(
        &self,
        provider: &ManagedAddress,
        client: &ManagedAddress,
        shared_amount_kwh: BigUint,
    ) {
        // check caller infos : prevent fraud
        let caller = self.blockchain().get_caller();
        require!(
            !self.modules(&caller).is_empty(),
            "forbidden : trying to share form non-module address"
        );
        require!(
            caller == self.provider(provider).get().sharing_module_address,
            "can't share electricity from provided provider address"
        );

        // check contract infos
        require!(
            self.contract_status(provider, client) == ContractStatus::Terminable,
            "can't share electricity after contract deadline"
        );

        require!(
            shared_amount_kwh > BigUint::zero(),
            "kwh shared amount can't be negative"
        );

        let current_contractors = (provider.clone(), client.clone());
        let esdt_amount_to_transfer = shared_amount_kwh * self.provider(provider).get().kwh_price;
        require!(
            esdt_amount_to_transfer
                <= self
                    .contract(&current_contractors)
                    .get()
                    .client_esdt_amount_locked,
            "forbidden : can't tranfer more than client locked amount"
        );

        // Reduce due amount from client "account"
        self.contract(&current_contractors)
            .update(|details| details.client_esdt_amount_locked -= esdt_amount_to_transfer.clone());
        // Add due amount to provider "account"
        self.provider(provider)
            .update(|details| details.due_amount += esdt_amount_to_transfer.clone());
    }

    #[endpoint]
    fn terminate_contract(&self, with_address: &ManagedAddress) {
        let caller = self.blockchain().get_caller();

        let mut provider = with_address;
        let mut client = &caller;

        // if caller is client, try to invert
        if self.contract(&(provider.clone(), client.clone())).is_empty() {
            provider = &caller;
            client = with_address;
        }

        require!(
            self.contract_status(provider, client) == ContractStatus::Terminated,
            "no active contract between you and provided address"
        );

        let contractors = (provider.clone(), client.clone());
        self.contract(&contractors)
            .update(|details| details.deadline = self.get_current_time());
    }

    #[view]
    fn contract_status(
        &self,
        provider: &ManagedAddress,
        client: &ManagedAddress,
    ) -> ContractStatus {
        let current_contractors = (provider.clone(), client.clone());

        require!(
            !self.contract(&current_contractors).is_empty(),
            "no contract found between caller and provided provider_address"
        );

        if self.get_current_time() < self.contract(&current_contractors).get().deadline {
            ContractStatus::Terminable
        } else if self
            .contract(&current_contractors)
            .get()
            .client_esdt_amount_locked
            > 0u32
        {
            ContractStatus::ClientEsdtAmountClaimable
        } else {
            ContractStatus::Terminated
        }
    }

    #[endpoint]
    fn claim_refund(&self, provider: &ManagedAddress) {
        match self.contract_status(provider, &self.blockchain().get_caller()) {
            ContractStatus::Terminable => sc_panic!("cannot claim before contract deadline"),
            ContractStatus::Terminated => sc_panic!("contract is terminated, no esdt to claim"),
            ContractStatus::ClientEsdtAmountClaimable => {
                let caller = self.blockchain().get_caller();
                let token_identifier = self.cf_token_identifier().get();
                let current_contractors = (provider.clone(), caller.clone());
                let due = self
                    .contract(&current_contractors)
                    .get()
                    .client_esdt_amount_locked;

                // effacer contrat entre provider et client
                self.contract(&current_contractors).clear();
                // rembourser <client>
                self.send().direct(&caller, &token_identifier, 0, &due);
            }
        }
    }

    #[view]
    fn provider_account_status(&self, provider: &ManagedAddress) -> ProviderAccountStatus {
        require!(
            !self.provider(provider).is_empty(),
            "provided address is not a provider"
        );

        if self.provider(provider).get().due_amount > 0u32 {
            ProviderAccountStatus::EsdtAmountClaimable
        } else {
            ProviderAccountStatus::Empty
        }
    }

    #[endpoint]
    fn claim_providers_due(&self) {
        let caller = self.blockchain().get_caller();

        match self.provider_account_status(&caller) {
            ProviderAccountStatus::Empty => sc_panic!("no esdt to claim"),
            ProviderAccountStatus::EsdtAmountClaimable => {
                let token_identifier = self.cf_token_identifier().get();
                let due = self.provider(&caller).get().due_amount;

                self.provider(&caller)
                    .update(|details| details.due_amount = BigUint::zero());
                self.send().direct(&caller, &token_identifier, 0, &due);
            }
        }
    }

    // PRIVATE

    fn get_current_time(&self) -> u64 {
        self.blockchain().get_block_timestamp()
    }

    // STORAGE

    // SingleValueMapper<provider_address>:{ sharing_module_address, deadline, kwh_price, due_amount }
    #[view(getProviderDetails)]
    #[storage_mapper("providerDetails")]
    fn provider(&self, provider: &ManagedAddress) -> SingleValueMapper<ProviderDetails<Self::Api>>;

    // SingleValueMapper<(provider_address, client_address)>:{ sc_deadline, client_esdt_amount_locked }
    // contractors : PROVIDER FIRST then client
    #[view(getContractDetails)]
    #[storage_mapper("contractDetails")]
    fn contract(
        &self,
        contractors: &(ManagedAddress, ManagedAddress),
    ) -> SingleValueMapper<ContractDetails<Self::Api>>;

    // SingleValueMapper<module_address>:amount_shared
    #[view(getModulesInfos)]
    #[storage_mapper("modulesInfos")]
    fn modules(&self, address: &ManagedAddress) -> SingleValueMapper<BigUint>;

    // SingleValueMapper<T>:esdt_token_id
    #[view(getContractTokenIdentifier)]
    #[storage_mapper("tokenIdentifier")]
    fn cf_token_identifier(&self) -> SingleValueMapper<EgldOrEsdtTokenIdentifier>;
}

use starknet::ContractAddress;

#[starknet::interface]
pub trait IToken<TContractState> {
    fn total_supply(self: @TContractState) -> u128;
    fn balance_of(self: @TContractState, owner: ContractAddress) -> u128;
    fn allowance(self: @TContractState, owner: ContractAddress, spender: ContractAddress) -> u128;
    fn approve(ref self: TContractState, spender: ContractAddress, amount: u128) -> bool;
    fn transfer(ref self: TContractState, recipient: ContractAddress, amount: u128) -> bool;
    fn transfer_from(
        ref self: TContractState, owner: ContractAddress, recipient: ContractAddress, amount: u128,
    ) -> bool;
}

#[starknet::interface]
pub trait IRegistry<TContractState> {
    fn last_seed(self: @TContractState) -> felt252;
    fn bump(ref self: TContractState, seed: felt252);
}

#[starknet::interface]
pub trait IPermissionedVault<TContractState> {
    fn balance_of(self: @TContractState, owner: ContractAddress, bucket: felt252) -> u128;
    fn grant_operator(ref self: TContractState, operator: ContractAddress);
    fn deposit(ref self: TContractState, bucket: felt252, amount: u128);
    fn withdraw(ref self: TContractState, bucket: felt252, amount: u128) -> bool;
}

#[starknet::interface]
pub trait IPortfolioRouter<TContractState> {
    fn configure(ref self: TContractState, token: ContractAddress, vault: ContractAddress);
    fn rebalance(ref self: TContractState, bucket: felt252, amount: u128) -> bool;
}

#[starknet::contract]
mod token {
    use super::{IRegistryDispatcher, IRegistryDispatcherTrait, IToken};
    use core::num::traits::Zero;
    use core::num::traits::CheckedAdd;
    use core::option::OptionTrait;
    use starknet::storage::{
        Map,
        StorageMapReadAccess,
        StorageMapWriteAccess,
        StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{ContractAddress, get_caller_address};

    #[storage]
    struct Storage {
        owner: ContractAddress,
        registry: ContractAddress,
        total_supply: u128,
        balances: Map<ContractAddress, u128>,
        allowances: Map<(ContractAddress, ContractAddress), u128>,
        allowance_nonce: Map<(ContractAddress, ContractAddress), u64>,
        permissions: Map<(ContractAddress, felt252), bool>,
        sync_permission_guard: u8,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
        Approval: Approval,
        Minted: Minted,
        RegistrySynced: RegistrySynced,
    }

    #[derive(Drop, starknet::Event)]
    struct Transfer {
        from: ContractAddress,
        to: ContractAddress,
        value: u128,
    }

    #[derive(Drop, starknet::Event)]
    struct Approval {
        owner: ContractAddress,
        spender: ContractAddress,
        value: u128,
        nonce: u64,
    }

    #[derive(Drop, starknet::Event)]
    struct Minted {
        recipient: ContractAddress,
        value: u128,
    }

    #[derive(Drop, starknet::Event)]
    struct RegistrySynced {
        registry: ContractAddress,
        caller: ContractAddress,
        seed: felt252,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState,
        owner: ContractAddress,
        registry: ContractAddress,
        initial_supply: u128,
    ) {
        assert(!owner.is_zero(), 'owner=0');
        self.owner.write(owner);
        self.registry.write(registry);
        self.total_supply.write(initial_supply);
        self.balances.write(owner, initial_supply);
    }

    #[abi(embed_v0)]
    impl TokenImpl of IToken<ContractState> {
        fn total_supply(self: @ContractState) -> u128 {
            self.total_supply.read()
        }

        fn balance_of(self: @ContractState, owner: ContractAddress) -> u128 {
            self.balances.read(owner)
        }

        fn allowance(self: @ContractState, owner: ContractAddress, spender: ContractAddress) -> u128 {
            self.allowances.read((owner, spender))
        }

        fn approve(ref self: ContractState, spender: ContractAddress, amount: u128) -> bool {
            assert(!spender.is_zero(), 'spender=0');
            let owner = get_caller_address();
            self.allowances.write((owner, spender), amount);
            let nonce = self.allowance_nonce.read((owner, spender)) + 1_u64;
            self.allowance_nonce.write((owner, spender), nonce);
            self.emit(Event::Approval(Approval {
                owner,
                spender,
                value: amount,
                nonce,
            }));
            true
        }

        fn transfer(ref self: ContractState, recipient: ContractAddress, amount: u128) -> bool {
            assert(!recipient.is_zero(), 'recipient=0');
            let sender = get_caller_address();
            let sender_balance = self.balances.read(sender);
            if sender_balance < amount {
                return false;
            }

            let recipient_balance = self.balances.read(recipient);
            let new_recipient_balance = OptionTrait::expect(
                recipient_balance.checked_add(amount), 'balance overflow',
            );
            self.balances.write(sender, sender_balance - amount);
            self.balances.write(recipient, new_recipient_balance);
            self.emit(Event::Transfer(Transfer {
                from: sender,
                to: recipient,
                value: amount,
            }));
            true
        }

        fn transfer_from(
            ref self: ContractState, owner: ContractAddress, recipient: ContractAddress, amount: u128,
        ) -> bool {
            assert(!owner.is_zero(), 'owner=0');
            assert(!recipient.is_zero(), 'recipient=0');
            let spender = get_caller_address();
            let allowed = self.allowances.read((owner, spender));
            if allowed < amount {
                return false;
            }

            let owner_balance = self.balances.read(owner);
            if owner_balance < amount {
                return false;
            }

            let recipient_balance = self.balances.read(recipient);
            let new_recipient_balance = OptionTrait::expect(
                recipient_balance.checked_add(amount), 'balance overflow',
            );
            self.balances.write(owner, owner_balance - amount);
            self.balances.write(recipient, new_recipient_balance);
            self.allowances.write((owner, spender), allowed - amount);
            self.emit(Event::Transfer(Transfer {
                from: owner,
                to: recipient,
                value: amount,
            }));
            true
        }
    }

    #[external(v0)]
    fn mint(ref self: ContractState, recipient: ContractAddress, amount: u128) {
        assert(self.owner.read() == get_caller_address(), 'not owner');
        assert(!recipient.is_zero(), 'recipient=0');
        let current_supply = self.total_supply.read();
        let new_supply = OptionTrait::expect(current_supply.checked_add(amount), 'supply overflow');
        self.total_supply.write(new_supply);

        let recipient_balance = self.balances.read(recipient);
        let new_balance = OptionTrait::expect(recipient_balance.checked_add(amount), 'balance overflow');
        self.balances.write(recipient, new_balance);
        self.emit(Event::Minted(Minted {
            recipient,
            value: amount,
        }));
    }

    #[external(v0)]
    fn sync_permission_seed(ref self: ContractState) {
        let registry = self.registry.read();
        assert(!registry.is_zero(), 'registry=0');
        let caller = get_caller_address();
        assert(self.sync_permission_guard.read() == 0_u8, 'reentrant');
        self.sync_permission_guard.write(1_u8);
        // Smoke fixture assumes registry is trusted, but this guard still prevents
        // accidental re-entrancy if registry logic is swapped during experiments.
        let seed = IRegistryDispatcher { contract_address: registry }.last_seed();
        self.permissions.write((caller, seed), true);
        self.emit(Event::RegistrySynced(RegistrySynced {
            registry,
            caller,
            seed,
        }));
        self.sync_permission_guard.write(0_u8);
    }
}

#[starknet::contract]
mod registry {
    use super::IRegistry;
    use starknet::storage::{
        Map,
        StorageMapReadAccess,
        StorageMapWriteAccess,
        StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{ContractAddress, get_caller_address};
    use core::num::traits::Zero;

    #[storage]
    struct Storage {
        admin: ContractAddress,
        last_seed: felt252,
        latest_by_caller: Map<ContractAddress, felt252>,
        history: Map<(ContractAddress, felt252), felt252>,
    }

    #[constructor]
    fn constructor(ref self: ContractState, admin: ContractAddress, initial_seed: felt252) {
        assert(!admin.is_zero(), 'admin=0');
        self.admin.write(admin);
        self.last_seed.write(initial_seed);
    }

    #[abi(embed_v0)]
    impl RegistryImpl of IRegistry<ContractState> {
        fn last_seed(self: @ContractState) -> felt252 {
            self.last_seed.read()
        }

        fn bump(ref self: ContractState, seed: felt252) {
            assert(self.admin.read() == get_caller_address(), 'not admin');
            let caller = get_caller_address();
            let previous = self.latest_by_caller.read(caller);
            self.last_seed.write(seed);
            self.latest_by_caller.write(caller, seed);
            self.history.write((caller, seed), previous);
        }
    }
}

#[starknet::contract]
mod permissioned_vault {
    use super::IPermissionedVault;
    use core::num::traits::CheckedAdd;
    use core::num::traits::Zero;
    use core::option::OptionTrait;
    use starknet::storage::{
        Map,
        StorageMapReadAccess,
        StorageMapWriteAccess,
        StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{ContractAddress, get_caller_address};

    #[storage]
    struct Storage {
        owner: ContractAddress,
        operators: Map<(ContractAddress, ContractAddress), bool>,
        balances: Map<(ContractAddress, felt252), u128>,
        totals_by_bucket: Map<felt252, u128>,
        audit_log: Map<(ContractAddress, felt252), felt252>,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress) {
        assert(!owner.is_zero(), 'owner=0');
        self.owner.write(owner);
    }

    #[abi(embed_v0)]
    impl PermissionedVaultImpl of IPermissionedVault<ContractState> {
        fn balance_of(self: @ContractState, owner: ContractAddress, bucket: felt252) -> u128 {
            self.balances.read((owner, bucket))
        }

        fn grant_operator(ref self: ContractState, operator: ContractAddress) {
            assert(self.owner.read() == get_caller_address(), 'not owner');
            assert(!operator.is_zero(), 'operator=0');
            self.operators.write((self.owner.read(), operator), true);
        }

        fn deposit(ref self: ContractState, bucket: felt252, amount: u128) {
            let caller = get_caller_address();
            let key = (caller, bucket);
            let before = self.balances.read(key);
            let after = OptionTrait::expect(before.checked_add(amount), 'balance overflow');
            self.balances.write(key, after);

            let total_before = self.totals_by_bucket.read(bucket);
            let total_after = OptionTrait::expect(total_before.checked_add(amount), 'total overflow');
            self.totals_by_bucket.write(bucket, total_after);
            self.audit_log.write((caller, bucket), bucket);
        }

        fn withdraw(ref self: ContractState, bucket: felt252, amount: u128) -> bool {
            let caller = get_caller_address();
            let owner = self.owner.read();
            if caller != owner && !self.operators.read((owner, caller)) {
                return false;
            }

            let key = (owner, bucket);
            let before = self.balances.read(key);
            if before < amount {
                return false;
            }
            self.balances.write(key, before - amount);

            let total_before = self.totals_by_bucket.read(bucket);
            self.totals_by_bucket.write(bucket, total_before - amount);
            self.audit_log.write((caller, bucket), bucket);
            true
        }
    }
}

#[starknet::contract]
mod portfolio_router {
    use super::{
        IPortfolioRouter,
        IPermissionedVaultDispatcher,
        IPermissionedVaultDispatcherTrait,
        ITokenDispatcher,
        ITokenDispatcherTrait,
    };
    use core::num::traits::Zero;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};

    #[storage]
    struct Storage {
        owner: ContractAddress,
        token: ContractAddress,
        vault: ContractAddress,
        last_actor: ContractAddress,
        last_bucket: felt252,
        last_amount: u128,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress) {
        assert(!owner.is_zero(), 'owner=0');
        self.owner.write(owner);
    }

    #[abi(embed_v0)]
    impl PortfolioRouterImpl of IPortfolioRouter<ContractState> {
        fn configure(ref self: ContractState, token: ContractAddress, vault: ContractAddress) {
            assert(self.owner.read() == get_caller_address(), 'not owner');
            assert(!token.is_zero(), 'token=0');
            assert(!vault.is_zero(), 'vault=0');
            self.token.write(token);
            self.vault.write(vault);
        }

        fn rebalance(ref self: ContractState, bucket: felt252, amount: u128) -> bool {
            let token = self.token.read();
            let vault = self.vault.read();
            if token.is_zero() || vault.is_zero() {
                return false;
            }

            let transferred = ITokenDispatcher { contract_address: token }.transfer(vault, amount);
            if !transferred {
                return false;
            }

            // Touch a second contract call path so fixture builds cover cross-contract dispatch.
            let deposited = IPermissionedVaultDispatcher { contract_address: vault }
                .withdraw(bucket, amount);
            self.last_actor.write(get_caller_address());
            self.last_bucket.write(bucket);
            self.last_amount.write(amount);
            deposited
        }
    }
}

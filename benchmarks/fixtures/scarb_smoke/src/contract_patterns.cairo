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

#[starknet::contract]
mod token {
    use super::IToken;
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
        total_supply: u128,
        balances: Map<ContractAddress, u128>,
        allowances: Map<(ContractAddress, ContractAddress), u128>,
        allowance_nonce: Map<(ContractAddress, ContractAddress), u64>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
        Approval: Approval,
        Minted: Minted,
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

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress, initial_supply: u128) {
        assert(!owner.is_zero(), 'owner=0');
        self.owner.write(owner);
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
}

use starknet::ContractAddress;

#[starknet::interface]
pub trait IToken<TContractState> {
    fn total_supply(self: @TContractState) -> u128;
    fn balance_of(self: @TContractState, owner: ContractAddress) -> u128;
    fn transfer(ref self: TContractState, recipient: ContractAddress, amount: u128) -> bool;
}

#[starknet::contract]
mod token {
    use super::IToken;
    use core::num::traits::Zero;
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
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
        Minted: Minted,
    }

    #[derive(Drop, starknet::Event)]
    struct Transfer {
        from: ContractAddress,
        to: ContractAddress,
        value: u128,
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

        fn transfer(ref self: ContractState, recipient: ContractAddress, amount: u128) -> bool {
            assert(!recipient.is_zero(), 'recipient=0');
            let sender = get_caller_address();
            let sender_balance = self.balances.read(sender);
            if sender_balance < amount {
                return false;
            }

            let recipient_balance = self.balances.read(recipient);
            self.balances.write(sender, sender_balance - amount);
            self.balances.write(recipient, recipient_balance + amount);
            self.emit(Event::Transfer(Transfer {
                from: sender,
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
        self.total_supply.write(current_supply + amount);

        let recipient_balance = self.balances.read(recipient);
        self.balances.write(recipient, recipient_balance + amount);
        self.emit(Event::Minted(Minted {
            recipient,
            value: amount,
        }));
    }
}

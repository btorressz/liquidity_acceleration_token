# liquidity_acceleration_token

# Liquidity Acceleration Token (LAT)

## Overview
 Liquidity Acceleration Token (LAT) is a Solana-based protocol designed to enhance Automated Market Maker (AMM) pool liquidity by dynamically rewarding traders and liquidity providers (LPs) based on execution speed, trading volume, and market activity. The protocol uses an epoch-based reward system to optimize token distribution, reduce slippage, and attract High-Frequency Trading (HFT) activity.

 ## Features

### 🔹 Dynamic LAT Rewards for Traders
- Traders earn LAT rewards based on trade execution frequency and volume.
- Uses an epoch-based reward system to prevent bot abuse and encourage fair distribution.
- Implements dynamic reward scaling, adjusting incentives based on market conditions.

### 🔹 Staking LAT for Enhanced Yield
- Liquidity providers can stake LAT to receive boosted rewards in AMM pools.
- Pools with high trading activity receive higher staking incentives.
- A 7-day vesting period prevents reward manipulation and ensures long-term commitment.

### 🔹 Liquidity Optimization Mechanism
- The protocol auto-adjusts liquidity allocation to pools with the highest trading activity.
- Encourages deep liquidity provisioning across Solana AMMs.
- Reduces slippage by strategically directing capital toward high-volume pairs.

### 🔹 Sybil Resistance & Flash Loan Protection
- Time-locked staking rewards to prevent flash loan exploitation.
- Vesting period on earned rewards to encourage long-term engagement.
- Requires traders to have an account older than X days before earning rewards.

## Smart Contract Implementation

### ✅ Core Functionalities

The protocol consists of the following Anchor-based Solana smart contract implementations:

#### Trade Reward System (`record_trade` / `claim_trade_rewards`)
- Traders accumulate pending LAT rewards based on execution frequency and volume.
- A claiming mechanism distributes accumulated rewards at the end of an epoch.
- Dynamic scaling applies higher incentives during low activity periods.

#### LAT Staking & Liquidity Provider Incentives (`stake_lat` / `claim_stake_rewards`)
- LPs stake LAT tokens to earn boosted staking rewards.
- Pools with high trading activity receive increased staking APY.
- Implements liquidity rebalancing to direct capital efficiently.

#### Liquidity Vault & Reward Distribution (`withdraw_stake`)
- LPs can withdraw staked LAT after the vesting period.
- Staking rewards are adjusted dynamically based on pool volume thresholds.
- Ensures fair distribution of LAT emissions to maintain sustainable growth.


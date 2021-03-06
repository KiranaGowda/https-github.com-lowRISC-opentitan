// Copyright lowRISC contributors.
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

class key_sideload_base_seq extends dv_base_seq #(
    .REQ         (key_sideload_item),
    .CFG_T       (key_sideload_agent_cfg),
    .SEQUENCER_T (key_sideload_sequencer)
  );
  `uvm_object_utils(key_sideload_base_seq)

  `uvm_object_new

  virtual task body();
    `uvm_fatal(`gtn, "Need to override this when you extend from this class!")
  endtask

endclass

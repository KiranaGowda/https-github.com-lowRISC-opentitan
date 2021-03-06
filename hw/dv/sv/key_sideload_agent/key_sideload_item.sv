// Copyright lowRISC contributors.
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

class key_sideload_item extends uvm_sequence_item;

  // random variables
  rand bit                            valid;
  rand bit [keymgr_pkg::KeyWidth-1:0] key0;
  rand bit [keymgr_pkg::KeyWidth-1:0] key1;
  rand int unsigned                   rsp_delay = 1;

  constraint rsp_delay_constraint_c {rsp_delay inside {[1:10]};}

  `uvm_object_utils_begin(key_sideload_item)
    `uvm_field_int(valid, UVM_DEFAULT)
    `uvm_field_int(key0,  UVM_DEFAULT)
    `uvm_field_int(key1,  UVM_DEFAULT)
    `uvm_field_int(rsp_delay,  UVM_DEFAULT)
  `uvm_object_utils_end

  `uvm_object_new

endclass

use burn::nn::conv::{Conv2d, Conv2dConfig};
use burn::nn::pool::{MaxPool2d, MaxPool2dConfig};
use burn::nn::{Dropout, DropoutConfig, Linear, LinearConfig, PaddingConfig2d, Relu};
use burn::prelude::*;

#[derive(Module, Debug)]
pub struct Net {
    pub c1: Conv2d,
    pub c2: Conv2d,
    pub c3: Conv2d,
    pub c4: Conv2d,
    pub c5: Conv2d,
    pub fc1: Linear,
    pub fc2: Linear,
    pool: MaxPool2d,
    drop: Dropout,
    act: Relu,
}

fn conv(i: usize, o: usize, d: &Device) -> Conv2d {
    Conv2dConfig::new([i, o], [3, 3]).with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1)).init(d)
}

impl Net {
    pub fn new(classes: usize, d: &Device) -> Self {
        Self {
            c1: conv(1, 32, d),
            c2: conv(32, 32, d),
            c3: conv(32, 64, d),
            c4: conv(64, 64, d),
            c5: conv(64, 128, d),
            fc1: LinearConfig::new(128 * 16, 256).init(d),
            fc2: LinearConfig::new(256, classes).init(d),
            pool: MaxPool2dConfig::new([2, 2]).with_strides([2, 2]).init(),
            drop: DropoutConfig::new(0.3).init(),
            act: Relu::new(),
        }
    }

    /// x: [batch, 1, 32, 32] -> logits [batch, classes]
    pub fn forward(&self, x: Tensor<4>) -> Tensor<2> {
        let x = self.act.forward(self.c1.forward(x));
        let x = self.pool.forward(self.act.forward(self.c2.forward(x)));
        let x = self.act.forward(self.c3.forward(x));
        let x = self.pool.forward(self.act.forward(self.c4.forward(x)));
        let x = self.pool.forward(self.act.forward(self.c5.forward(x)));
        let x = x.flatten::<2>(1, 3);
        let x = self.drop.forward(x);
        self.fc2.forward(self.act.forward(self.fc1.forward(x)))
    }
}
